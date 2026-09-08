import Fastify, { type FastifyInstance } from 'fastify'
import { timingSafeEqual } from 'node:crypto'
import { z } from 'zod'
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
      const botUserId = await options.botUserId?.()
      const mentions = mentionsFromActivity(payload, {
        ...(botUserId ? { botUserId } : {}),
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
    return reply.code(200).send(result)
  })

  return app
}
