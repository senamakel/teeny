import Fastify, { type FastifyInstance } from 'fastify'
import { timingSafeEqual } from 'node:crypto'
import { z } from 'zod'
import { redactReflection, type Journal } from './journal/journal.js'
import type { MentionQueue } from './queue/mentions.js'
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

  app.post('/agent/run', async (request, reply) => {
    if (!options.agent) {
      return reply.code(503).send({ error: 'the agent is not configured' })
    }
    if (!options.agentToken) {
      return reply.code(503).send({ error: 'AGENT_API_TOKEN is not set' })
    }
    const header = request.headers.authorization
    const provided = header?.startsWith('Bearer ') ? header.slice('Bearer '.length) : undefined
    if (!matchesToken(provided, options.agentToken)) {
      return reply.code(401).send({ error: 'unauthorized' })
    }

    const parsed = agentRequestSchema.safeParse(request.body)
    if (!parsed.success) {
      return reply.code(400).send({ error: 'task is required', details: parsed.error.issues })
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
