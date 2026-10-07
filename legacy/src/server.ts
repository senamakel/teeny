import Fastify, { type FastifyInstance, type FastifyReply, type FastifyRequest } from 'fastify'
import { timingSafeEqual } from 'node:crypto'
import { z } from 'zod'
import { redactReflection, type Journal } from './journal/journal.js'
import type { MentionQueue } from './queue/mentions.js'
import { dashboardHtml } from './safety/dashboard.js'
import type { SafetyGate } from './safety/gate.js'
import type { Hold, HoldLog, HoldStatus } from './safety/holds.js'
import { reviewHold } from './safety/review.js'
import { renderCritique, type Critic } from './critic/algo.js'
import type { Agent, Memory } from './types.js'
import { crcResponse, isValidWebhookSignature } from './webhooks/activity.js'
import type { People } from './cadence/people.js'
import type { WebhookForwarder } from './webhooks/forward.js'
import { ActivityIntake } from './webhooks/intake.js'
import { redactInference, type InferenceLedger, type Lane } from './ledger/ledger.js'
import type { BrainReader } from './ledger/brain.js'
import type { ArticleStore } from './article/store.js'

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

const critiqueRequestSchema = z.object({
  kind: z.enum(['post', 'quote', 'reply', 'caption', 'script', 'thread', 'other']).default('post'),
  text: z.string().min(1).max(8_000),
  media: z.string().max(600).optional(),
  context: z.string().max(2_000).optional(),
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
  /**
   * The inference ledger, served at GET /ledger: every model call, and the
   * running token count, for the page's counter. Same network as /mind.
   */
  ledger?: InferenceLedger
  /** The brain's checkout, served at GET /brain: roster, workflows, the latest runs. */
  brain?: BrainReader
  /** The articles on disk, served at GET /articles for the site: the published ones, or every rehearsal with `?all=1`. */
  articles?: ArticleStore
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
  /** The ranker's reader, served at POST /critique behind the same token as the task endpoint. */
  critic?: Critic
  /** Where a copy of every verified X delivery is sent, after X has been answered. */
  forwarder?: WebhookForwarder
  /** Who interacts with the account, fed from every delivery. */
  people?: Pick<People, 'observe'>
  /**
   * Where a verified delivery goes. Shared with the activity stream reader so
   * an event both transports carry is handled once; built from the options
   * above when nothing is passed.
   */
  intake?: ActivityIntake
}): FastifyInstance {
  const app = Fastify({ logger: options.logger ?? false })
  const intake =
    options.intake ??
    new ActivityIntake({
      memory: options.memory,
      logger: app.log,
      ...(options.queue ? { queue: options.queue } : {}),
      ...(options.botUserId ? { botUserId: options.botUserId } : {}),
      ...(options.botUsername ? { botUsername: options.botUsername } : {}),
      ...(options.people ? { people: options.people } : {}),
    })

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
    const [reflections, counts, totals] = await Promise.all([
      options.mind.journal.recent(limit),
      options.queue ? options.queue.counts().catch(() => undefined) : Promise.resolve(undefined),
      options.ledger ? options.ledger.totals().catch(() => undefined) : Promise.resolve(undefined),
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
      ...(totals ? { totals } : {}),
      reflections: reflections.map(redactReflection),
    }
  })

  app.get('/ledger', async (request, reply) => {
    if (!options.ledger) return reply.code(503).send({ error: 'no inference ledger is configured' })
    const query = request.query as { limit?: string; lane?: string; before?: string }
    const limit = Math.min(Math.max(Number(query.limit) || 50, 1), 200)
    const lane = query.lane === 'mascot' || query.lane === 'brain' ? (query.lane as Lane) : undefined
    const before = typeof query.before === 'string' && query.before ? query.before : undefined
    const [totals, entries] = await Promise.all([
      options.ledger.totals(),
      options.ledger.recent(limit, { ...(lane ? { lane } : {}), ...(before ? { before } : {}) }),
    ])
    return {
      source: 'live',
      generatedAt: new Date().toISOString(),
      totals,
      entries: entries.map(redactInference),
    }
  })

  app.get('/brain', async (request, reply) => {
    if (!options.brain) return reply.code(503).send({ error: 'no brain is mounted' })
    const query = request.query as { runs?: string; steps?: string }
    return options.brain.snapshot({
      runs: Math.min(Math.max(Number(query.runs) || 40, 1), 200),
      steps: Math.min(Math.max(Number(query.steps) || 300, 1), 1000),
    })
  })

  app.get('/articles', async (request, reply) => {
    if (!options.articles) return reply.code(503).send({ error: 'no article store is configured' })
    const query = request.query as { all?: string; limit?: string }
    const articles = await options.articles.list({
      all: query.all === '1' || query.all === 'true',
      limit: Math.min(Math.max(Number(query.limit) || 50, 1), 200),
    })
    return { source: 'live', generatedAt: new Date().toISOString(), articles }
  })

  app.get('/articles/:slug', async (request, reply) => {
    if (!options.articles) return reply.code(503).send({ error: 'no article store is configured' })
    const { slug } = request.params as { slug: string }
    const query = request.query as { all?: string }
    const article = await options.articles.get(slug)
    if (!article) return reply.code(404).send({ error: 'no such article' })
    if (query.all === '1' || query.all === 'true') return article
    // The operator's detail stays with the operator; the site gets the piece.
    const { problems: _problems, error: _error, critique: _critique, proof: _proof, judge: _judge, contentState: _contentState, ...published } = article
    return published
  })

  app.get('/articles/:slug/media/:file', async (request, reply) => {
    if (!options.articles) return reply.code(503).send({ error: 'no article store is configured' })
    const { slug, file } = request.params as { slug: string; file: string }
    const media = await options.articles.media(slug, file)
    if (!media) return reply.code(404).send({ error: 'no such file' })
    return reply.type(media.mimeType).header('cache-control', 'public, max-age=86400').send(Buffer.from(media.data))
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

    // The copy goes out first and is not awaited: the receiver's clock is
    // its own, and X's is the one this handler answers to.
    if (options.forwarder) {
      const copy = request.rawBody
      const header = Array.isArray(signature) ? signature[0] : signature
      void options.forwarder.forward(copy, header).catch((error: unknown) => {
        request.log.warn({ err: error }, 'webhook copy failed')
      })
    }

    // X retries a delivery it does not see acknowledged quickly, so the work is
    // queued rather than done here: the reply itself takes a model call and
    // several seconds, which is far longer than a webhook may hold the line.
    // The intake answers once the queue has it; memory is written behind.
    await intake.deliver(payload, 'webhook')
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

  /**
   * Feedback on anything about to be published -- a post, a caption, a clip
   * script, a thread -- from the reader that knows the ranker's weights. The
   * text is the operator's and nothing is published, so it is not put
   * through the content filter; the critic's own brief treats it as data.
   */
  app.post('/critique', async (request, reply) => {
    if (!options.critic) {
      return reply.code(503).send({ error: 'the critic is not configured' })
    }
    if (!authorised(request, reply)) return reply
    const parsed = critiqueRequestSchema.safeParse(request.body)
    if (!parsed.success) {
      return reply.code(400).send({ error: 'text is required', details: parsed.error.issues })
    }
    try {
      const { kind, text, media, context } = parsed.data
      const critique = await options.critic.critique({
        kind,
        text,
        ...(media ? { media } : {}),
        ...(context ? { context } : {}),
      })
      return reply.code(200).send({ ...critique, rendered: renderCritique(critique) })
    } catch (error) {
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
