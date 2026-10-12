import { describe, expect, it, vi } from 'vitest'
import { ActivityStream } from '../src/clients/activity-stream.js'

const BOT = '2094854245468000256'

function logger() {
  return {
    debug: vi.fn(),
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
  } as never
}

/** One streamed response: the given chunks, then end of stream. */
function response(chunks: string[], init: { status?: number } = {}): Response {
  const encoder = new TextEncoder()
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk))
      controller.close()
    },
  })
  return new Response(body, { status: init.status ?? 200 })
}

function event(eventType: string, id: string, extra: Record<string, unknown> = {}) {
  return JSON.stringify({
    data: { event_uuid: id, event_type: eventType, filter: { user_id: BOT }, payload: { id, ...extra } },
  })
}

describe('activity stream', () => {
  it('hands each event line to the intake and ignores keep-alives and partial chunks', async () => {
    const onEvent = vi.fn().mockResolvedValue(undefined)
    const first = event('post.mention.create', '900', { text: 'hi @teenydroid', author_id: '42' })
    const second = event('like.create', '901')
    // Two events split across chunks, keep-alive newlines between them.
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(response(['\r\n', first.slice(0, 20), first.slice(20) + '\r\n\r\n', second + '\r\n']))
    const stream = new ActivityStream({ bearerToken: 't', logger: logger(), onEvent, fetch, sleep: async () => undefined })

    // The run reconnects forever, so stop it after the first stream ends.
    onEvent.mockImplementation(async (payload: { data: { event_type: string } }) => {
      if (payload.data.event_type === 'like.create') stream.stop()
    })
    await stream.run()

    expect(onEvent).toHaveBeenCalledTimes(2)
    expect(onEvent.mock.calls[0]?.[0]).toEqual(JSON.parse(first))
    expect(onEvent.mock.calls[1]?.[0]).toEqual(JSON.parse(second))
    expect(fetch).toHaveBeenCalledWith(
      'https://api.x.com/2/activity/stream',
      expect.objectContaining({ headers: { authorization: 'Bearer t' } }),
    )
  })

  it('reconnects after the stream ends, and backs off on an HTTP error', async () => {
    const onEvent = vi.fn().mockResolvedValue(undefined)
    const delays: number[] = []
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(response([event('like.create', '1') + '\n']))
      .mockResolvedValueOnce(response(['{"title":"Too Many Requests"}'], { status: 429 }))
      .mockResolvedValueOnce(response(['{"title":"nope"}'], { status: 503 }))
      .mockResolvedValueOnce(response([event('like.create', '2') + '\n']))
    const stream = new ActivityStream({
      bearerToken: 't',
      logger: logger(),
      onEvent,
      fetch,
      sleep: async (ms) => {
        delays.push(ms)
      },
    })
    onEvent.mockImplementation(async (payload: { data: { event_uuid: string } }) => {
      if (payload.data.event_uuid === '2') stream.stop()
    })
    await stream.run()

    expect(onEvent).toHaveBeenCalledTimes(2)
    // A 429 waits a minute; the 503 that follows is the second consecutive
    // failure, so the HTTP backoff has doubled once.
    expect(delays).toEqual([60_000, 10_000])
  })

  it('does not let one failed event stop the stream', async () => {
    const onEvent = vi
      .fn()
      .mockRejectedValueOnce(new Error('mongo is away'))
      .mockImplementation(async () => {
        stream.stop()
      })
    const fetch = vi.fn().mockResolvedValueOnce(response([event('like.create', '1'), '\n', event('like.create', '2'), '\n']))
    const stream = new ActivityStream({ bearerToken: 't', logger: logger(), onEvent, fetch, sleep: async () => undefined })
    await stream.run()

    expect(onEvent).toHaveBeenCalledTimes(2)
  })

  it('gives up after the configured number of consecutive failures', async () => {
    const fetch = vi.fn().mockRejectedValue(new Error('ECONNRESET'))
    const log = logger()
    const stream = new ActivityStream({
      bearerToken: 't',
      logger: log,
      onEvent: vi.fn(),
      fetch,
      sleep: async () => undefined,
      maxAttempts: 3,
    })
    await stream.run()

    expect(fetch).toHaveBeenCalledTimes(3)
  })
})
