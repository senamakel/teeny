import Fastify, { type FastifyInstance, type FastifyReply, type FastifyRequest } from 'fastify'
import { timingSafeEqual } from 'node:crypto'
import { z } from 'zod'
import { redactReflection, type Journal } from './journal/journal.js'
import type { MentionQueue } from './queue/mentions.js'
import { dashboardHtml } from './safety/dashboard.js'
import type { SafetyGate } from './safety/gate.js'
import type { Hold, HoldLog, HoldStatus } from './safety/holds.js'
import { reviewHold } from './safety/review.js'
import type { Agent, Memory } from './types.js'
import {
  captureWebhookActivity,
  crcResponse,
  isValidWebhookSignature,
  mentionsFromActivity,
} from './webhooks/activity.js'

declare module 'fastify' {
  interface FastifyRequest {
    rawBody?: Buffer
  }
}

const agentRequestSchema = z.object({
  task: z.string().min(1).max(4_000),
  threadId: z.string().min(1).max(200).optional(),
})

function matchesToken(provided: string | undefined, expected: string): boolean {
  if (!provided) return false
  const a = Buffer.from(provided)
  const b = Buffer.from(expected)
  return a.length === b.length && timingSafeEqual(a, b)
}

/**
 * The operator's token, from either shape a client sends it in. A script
 * sends `Bearer`; a browser opening the review page sends nothing, gets a
 * 401 with a challenge, and comes back with `Basic` and the token as the
 * password, which is the one way to put a token in front of a plain page
 * without building a login.
 */
function tokenFrom(header: string | undefined): string | undefined {
  if (!header) return undefined
  if (header.startsWith('Bearer ')) return header.slice('Bearer '.length)
  if (header.startsWith('Basic ')) {
    const decoded = Buffer.from(header.slice('Basic '.length), 'base64').toString('utf8')
    const colon = decoded.indexOf(':')
    return colon === -1 ? decoded : decoded.slice(colon + 1)
  }
  return undefined
}

const decisionSchema = z.object({
  decision: z.enum(['approved', 'rejected']),
  note: z.string().max(500).optional(),
})

export function buildServer(options: {
  memory: Memory
  consumerSecret: string
  logger?: boolean | { level: string }
  agent?: Agent
  agentToken?: string
  queue?: MentionQueue
  /** The account's own id, so its own posts are not answered. */
  botUserId?: () => Promise<string | undefined>
  botUsername?: string
  /**
   * The mind, served at GET /mind for the landing page. Read over the compose
   * network, not the public vhost: nginx does not route it, and the page is
   * what redacts and presents it.
   */
  mind?: {
    journal: Journal
    status: { mode: string; model: string; dryRun: boolean }
  }
  /** The content filter. An ad-hoc task it holds is refused before the agent sees it. */
  safety?: Pick<SafetyGate, 'inbound' | 'refreshApprovedLinks'>
  /**
   * The review side of the filter: everything it held, served at /safety
   * for an operator to approve or reject. Same token as the task endpoint.
   */
  review?: {
    holds: HoldLog
    /** Publishes an approved draft. Absent, approving a draft only records it. */
    release?: (hold: Hold) => Promise<string>
  }
}): FastifyInstance {
  const app = Fastify({ logger: options.logger ?? false })

  app.addContentTypeParser(
    'application/json',
    { parseAs: 'buffer' },
    (request, body, done) => {
      request.rawBody = Buffer.isBuffer(body) ? body : Buffer.from(body)
      try {
        done(null, JSON.parse(request.rawBody.toString('utf8')))
      } catch (error) {
        done(error as Error, undefined)
      }
    },
  )

  app.get('/health', async () => ({ ok: true }))

  app.get('/queue', async (_request, reply) => {
    if (!options.queue) return reply.code(503).send({ error: 'no mention queue is configured' })
    return { counts: await options.queue.counts() }
  })

  app.get('/mind', async (request, reply) => {
    if (!options.mind) return reply.code(503).send({ error: 'no journal is configured' })
    const query = request.query as { limit?: string }
    const limit = Math.min(Math.max(Number(query.limit) || 40, 1), 200)
    const [reflections, counts] = await Promise.all([
      options.mind.journal.recent(limit),
      options.queue ? options.queue.counts().catch(() => undefined) : Promise.resolve(undefined),
    ])
    return {
      source: 'live',
      generatedAt: new Date().toISOString(),
      status: {
        ...options.mind.status,
        queue: {
          pending: counts?.pending ?? 0,
          done: counts?.replied ?? 0,
          failed: counts?.failed ?? 0,
        },
      },
      reflections: reflections.map(redactReflection),
    }
  })

  app.get('/webhooks/x', async (request, reply) => {
    const query = request.query as { crc_token?: string }
    if (!query.crc_token) return reply.code(400).send({ error: 'crc_token is required' })
    return { response_token: crcResponse(query.crc_token, options.consumerSecret) }
  })

  app.post('/webhooks/x', async (request, reply) => {
    const signature = request.headers['x-twitter-webhooks-signature']
    if (
      !request.rawBody ||
      !isValidWebhookSignature(
        request.rawBody,
        Array.isArray(signature) ? signature[0] : signature,
        options.consumerSecret,
      )
    ) {
      return reply.code(401).send({ error: 'invalid webhook signature' })
    }

    const payload = request.body as Record<string, unknown>

    // X retries a delivery it does not see acknowledged quickly, so the work is
    // queued rather than done here: the reply itself takes a model call and
    // several seconds, which is far longer than a webhook may hold the line.
    if (options.queue) {
      // Identity first, and nothing is queued without it: a delivery is mostly
      // the account's own activity, and queuing that is how a self-reply loop
      // starts. The cron poll is the safety net for anything dropped here.
      let botUserId: string | undefined
      try {
        botUserId = await options.botUserId?.()
      } catch (error) {
        request.log.error({ err: error }, 'could not identify the account; queued nothing')
      }
      if (!botUserId) {
        request.log.error('no account id available; refusing to queue webhook activity')
        return reply.code(200).send({ ok: true })
      }
      const mentions = mentionsFromActivity(payload, {
        botUserId,
        botUsername: options.botUsername ?? '',
      })
      if (mentions.length > 0) {
        const cached = await options.queue.cache(mentions)
        request.log.info(
          { delivered: mentions.length, cached },
          'queued mentions from an activity webhook',
        )
      }
    }

    await captureWebhookActivity(options.memory, payload)
    return reply.code(200).send({ ok: true })
  })

  /** True when the request carries the operator token; otherwise it has already been answered. */
  const authorised = (request: FastifyRequest, reply: FastifyReply, browser = false): boolean => {
    if (!options.agentToken) {
      reply.code(503).header('content-type', 'application/json').send({ error: 'AGENT_API_TOKEN is not set' })
      return false
    }
    if (matchesToken(tokenFrom(request.headers.authorization), options.agentToken)) return true
    reply
      .code(401)
      .header('www-authenticate', browser ? 'Basic realm="teeny", charset="UTF-8"' : 'Bearer')
      .send({ error: 'unauthorized' })
    return false
  }

  app.get('/safety', async (request, reply) => {
    if (!options.review) return reply.code(503).send({ error: 'no hold log is configured' })
    if (!authorised(request, reply, true)) return reply
    return reply.header('content-type', 'text/html; charset=utf-8').send(dashboardHtml())
  })

  app.get('/safety/holds', async (request, reply) => {
    if (!options.review) return reply.code(503).send({ error: 'no hold log is configured' })
    if (!authorised(request, reply, true)) return reply
    const query = request.query as { status?: string; limit?: string }
    const status = (['pending', 'approved', 'rejected'] as HoldStatus[]).find((s) => s === query.status)
    const limit = Math.min(Math.max(Number(query.limit) || 100, 1), 500)
    const [holds, counts] = await Promise.all([
      options.review.holds.list({ ...(status ? { status } : {}), limit }),
      options.review.holds.counts(),
    ])
    return { counts, holds }
  })

  app.post('/safety/holds/:id/decision', async (request, reply) => {
    if (!options.review) return reply.code(503).send({ error: 'no hold log is configured' })
    if (!authorised(request, reply, true)) return reply
    const parsed = decisionSchema.safeParse(request.body)
    if (!parsed.success) {
      return reply.code(400).send({ error: 'decision must be approved or rejected', details: parsed.error.issues })
    }
    const { id } = request.params as { id: string }
    try {
      const hold = await reviewHold(
        id,
        parsed.data.decision,
        {
          holds: options.review.holds,
          ...(options.queue ? { requeue: (postId: string) => options.queue!.requeueApproved(postId) } : {}),
          ...(options.review.release ? { release: options.review.release } : {}),
          ...(options.safety?.refreshApprovedLinks
            ? { refreshLinks: () => options.safety!.refreshApprovedLinks() }
            : {}),
        },
        parsed.data.note,
      )
      if (!hold) return reply.code(404).send({ error: 'no such hold' })
      request.log.info({ holdId: id, decision: parsed.data.decision, effect: hold.effect }, 'reviewed a hold')
      return hold
    } catch (error) {
      request.log.error({ err: error, holdId: id }, 'could not act on a hold decision')
      return reply.code(502).send({ error: error instanceof Error ? error.message : String(error) })
    }
  })

  app.post('/agent/run', async (request, reply) => {
    if (!options.agent) {
      return reply.code(503).send({ error: 'the agent is not configured' })
    }
    if (!authorised(request, reply)) return reply

    const parsed = agentRequestSchema.safeParse(request.body)
    if (!parsed.success) {
      return reply.code(400).send({ error: 'task is required', details: parsed.error.issues })
    }

    // The operator is trusted with the token, not with the filter: a task is
    // a message like any other, and the rules the account keeps for strangers
    // are the rules it keeps for everybody.
    const held = await options.safety?.inbound(parsed.data.task, { surface: 'task' })
    if (held && !held.ok) {
      return reply.code(422).send({ error: 'the task was held by the content filter', category: held.category })
    }

    const result = await options.agent.run({
      task: parsed.data.task,
      ...(parsed.data.threadId ? { threadId: parsed.data.threadId } : {}),
    })
    // An ad-hoc task is a turn of the mind like any other. The task text is
    // the operator's, so it is journaled as what came in.
    if (options.mind) {
      await options.mind.journal
        .record({
          trigger: 'task',
          incoming: parsed.data.task,
          tone: 'neutral',
          recalled: [],
          tools: result.steps.map((step) => ({ name: step.tool, result: step.result ?? '' })),
          thinking: '',
          said: result.output,
          declined: [],
          outcome: result.steps.some((step) => step.tool === 'x_post') ? 'posted' : 'held',
          outcomeNote: 'ad-hoc task via /agent/run',
        })
        .catch((error: unknown) => request.log.warn({ err: error }, 'could not journal the task'))
    }
    return reply.code(200).send(result)
  })

  return app
}
