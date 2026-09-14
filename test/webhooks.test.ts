import { createHmac } from 'node:crypto'
import { describe, expect, it, vi } from 'vitest'
import { buildServer } from '../src/server.js'
import type { Memory } from '../src/types.js'

function memory(): Memory {
  return {
    captureActivity: vi.fn().mockResolvedValue(undefined),
    captureMention: vi.fn().mockResolvedValue(undefined),
    recall: vi.fn().mockResolvedValue(''),
    captureReply: vi.fn().mockResolvedValue(undefined),
  }
}

describe('X webhook', () => {
  it('answers challenge-response checks', async () => {
    const app = buildServer({ memory: memory(), consumerSecret: 'secret' })
    const response = await app.inject({ method: 'GET', url: '/webhooks/x?crc_token=challenge' })
    const digest = createHmac('sha256', 'secret').update('challenge').digest('base64')

    expect(response.statusCode).toBe(200)
    expect(response.json()).toEqual({ response_token: `sha256=${digest}` })
    await app.close()
  })

  it('rejects unsigned events and captures signed activity', async () => {
    const store = memory()
    const app = buildServer({ memory: store, consumerSecret: 'secret' })
    const payload = JSON.stringify({
      tweet_create_events: [{ id_str: '123', text: 'hello' }],
      favorite_events: [{ id: '456' }],
    })

    const unsigned = await app.inject({
      method: 'POST',
      url: '/webhooks/x',
      headers: { 'content-type': 'application/json' },
      payload,
    })
    expect(unsigned.statusCode).toBe(401)

    const signature = `sha256=${createHmac('sha256', 'secret').update(payload).digest('base64')}`
    const signed = await app.inject({
      method: 'POST',
      url: '/webhooks/x',
      headers: {
        'content-type': 'application/json',
        'x-twitter-webhooks-signature': signature,
      },
      payload,
    })
    expect(signed.statusCode).toBe(200)

    // Only the non-post event reaches memory. A post goes to the queue and is
    // captured later as a chat turn attributed to its author, so writing the
    // raw delivery here too would store the same conversation twice, once
    // unattributed under the account's own id.
    await vi.waitFor(() => expect(store.captureActivity).toHaveBeenCalledTimes(1))
    expect(store.captureActivity).toHaveBeenCalledWith('favorite_events', '456', { id: '456' })
    await app.close()
  })
})

describe('activity deliveries become queued mentions', () => {
  const BOT = '2094854245468000256'

  function delivery(events: unknown[]): string {
    return JSON.stringify({ for_user_id: BOT, tweet_create_events: events })
  }

  function signed(payload: string) {
    return `sha256=${createHmac('sha256', 'secret').update(payload).digest('base64')}`
  }

  function queue() {
    return {
      cache: vi.fn().mockResolvedValue(1),
      takePending: vi.fn(),
      markReplied: vi.fn(),
      markFailed: vi.fn(),
      requeueApproved: vi.fn(),
      markSkipped: vi.fn().mockResolvedValue(undefined),
      markLiked: vi.fn().mockResolvedValue(undefined),
      countReplied: vi.fn().mockResolvedValue(0),
      counts: vi.fn(),
      close: vi.fn(),
    }
  }

  async function post(
    payload: string,
    mentions: ReturnType<typeof queue>,
    botUserId: () => Promise<string | undefined> = async () => BOT,
  ) {
    const app = buildServer({
      memory: memory(),
      consumerSecret: 'secret',
      queue: mentions,
      botUserId,
      botUsername: 'TinyMascot',
    })
    const response = await app.inject({
      method: 'POST',
      url: '/webhooks/x',
      headers: {
        'content-type': 'application/json',
        'x-twitter-webhooks-signature': signed(payload),
      },
      payload,
    })
    await app.close()
    return response
  }

  it('queues a mention, using the full text of a truncated post', async () => {
    const mentions = queue()
    const payload = delivery([
      {
        id_str: '900',
        text: 'hey @TinyMascot this is going to be cut off at one hundred and forty…',
        truncated: true,
        extended_tweet: { full_text: 'hey @TinyMascot the whole thing' },
        created_at: 'Wed Sep 02 13:30:16 +0000 2026',
        user: { id_str: '42', screen_name: 'someone' },
      },
    ])

    expect((await post(payload, mentions)).statusCode).toBe(200)
    expect(mentions.cache).toHaveBeenCalledWith([
      {
        id: '900',
        text: 'hey @TinyMascot the whole thing',
        authorId: '42',
        authorUsername: 'someone',
        createdAt: '2026-09-02T13:30:16.000Z',
      },
    ])
  })

  it('never queues the account\'s own posts, which would be a reply loop', async () => {
    const mentions = queue()
    const payload = delivery([
      { id_str: '901', text: 'gm from the bot', user: { id_str: BOT, screen_name: 'TinyMascot' } },
    ])

    expect((await post(payload, mentions)).statusCode).toBe(200)
    expect(mentions.cache).not.toHaveBeenCalled()
  })

  // The test above passes an id that always resolves, which is exactly what
  // production stopped doing: the account was renamed, the id was looked up by
  // the old handle, and the lookup returned nothing. Every self-check keyed on
  // that id went quiet at once and the account answered its own post.
  it('queues nothing when the account cannot be identified', async () => {
    const mentions = queue()
    const payload = delivery([
      { id_str: '901', text: 'gm from the bot', user: { id_str: BOT, screen_name: 'teenydroid' } },
    ])

    const response = await post(payload, mentions, async () => undefined)

    expect(response.statusCode).toBe(200)
    expect(mentions.cache).not.toHaveBeenCalled()
  })

  it('queues nothing when identifying the account fails outright', async () => {
    const mentions = queue()
    const payload = delivery([
      { id_str: '902', text: 'gm from the bot', user: { id_str: BOT, screen_name: 'teenydroid' } },
    ])

    const response = await post(payload, mentions, () =>
      Promise.reject(new Error('Could not find user with username: [TinyMascot].')),
    )

    // Still 200: X retries anything else until it gives up and disables the
    // webhook, and the cron poll is the safety net for what was dropped.
    expect(response.statusCode).toBe(200)
    expect(mentions.cache).not.toHaveBeenCalled()
  })

  it('drops a deep delivery carrying only the handle X prefixed for it', async () => {
    const mentions = queue()
    const payload = delivery([
      {
        id_str: '902',
        text: '@iamgaurangdesai @TinyMascot wen ser',
        in_reply_to_user_id_str: '999',
        user: { id_str: '42', screen_name: 'someone' },
      },
    ])

    expect((await post(payload, mentions)).statusCode).toBe(200)
    expect(mentions.cache).not.toHaveBeenCalled()
  })

  it('keeps a deep delivery where somebody typed the handle', async () => {
    const mentions = queue()
    const payload = delivery([
      {
        id_str: '903',
        text: '@senamakel Oh hi fren @TinyMascot',
        in_reply_to_user_id_str: '999',
        user: { id_str: '42', screen_name: 'coldcoleonx' },
      },
    ])

    expect((await post(payload, mentions)).statusCode).toBe(200)
    expect(mentions.cache).toHaveBeenCalledWith([
      expect.objectContaining({ id: '903', authorUsername: 'coldcoleonx' }),
    ])
  })
})

describe('retweets', () => {
  const BOT = '2094854245468000256'

  it('never queues a retweet of the account, which would be a self-reply', async () => {
    const mentions = {
      cache: vi.fn().mockResolvedValue(1),
      takePending: vi.fn(),
      markReplied: vi.fn(),
      markFailed: vi.fn(),
      requeueApproved: vi.fn(),
      markSkipped: vi.fn().mockResolvedValue(undefined),
      markLiked: vi.fn().mockResolvedValue(undefined),
      countReplied: vi.fn().mockResolvedValue(0),
      counts: vi.fn(),
      close: vi.fn(),
    }
    const payload = JSON.stringify({
      for_user_id: BOT,
      tweet_create_events: [
        {
          id_str: '2095151478851633461',
          text: 'RT @TinyMascot: @senamakel alive and tiny as ever, sena',
          retweeted_status: { id_str: '2095142265874231486' },
          user: { id_str: '999', screen_name: 'Sorin_016' },
        },
      ],
    })
    const app = buildServer({
      memory: memory(),
      consumerSecret: 'secret',
      queue: mentions,
      botUserId: async () => BOT,
      botUsername: 'TinyMascot',
    })
    const response = await app.inject({
      method: 'POST',
      url: '/webhooks/x',
      headers: {
        'content-type': 'application/json',
        'x-twitter-webhooks-signature': `sha256=${createHmac('sha256', 'secret')
          .update(payload)
          .digest('base64')}`,
      },
      payload,
    })

    expect(response.statusCode).toBe(200)
    expect(mentions.cache).not.toHaveBeenCalled()
    await app.close()
  })
})

describe('forwarding a delivery', () => {
  const payload = JSON.stringify({ favorite_events: [{ id: '456' }] })
  const signature = `sha256=${createHmac('sha256', 'secret').update(payload).digest('base64')}`

  it('sends a verified delivery on, body and signature untouched, after answering X', async () => {
    const forward = vi.fn().mockResolvedValue(undefined)
    const app = buildServer({ memory: memory(), consumerSecret: 'secret', forwarder: { forward } })
    const response = await app.inject({
      method: 'POST',
      url: '/webhooks/x',
      headers: { 'content-type': 'application/json', 'x-twitter-webhooks-signature': signature },
      payload,
    })
    expect(response.statusCode).toBe(200)
    expect(forward).toHaveBeenCalledTimes(1)
    const [body, header] = forward.mock.calls[0] as [Buffer, string]
    expect(body.toString('utf8')).toBe(payload)
    expect(header).toBe(signature)
    await app.close()
  })

  it('forwards nothing X did not sign', async () => {
    const forward = vi.fn().mockResolvedValue(undefined)
    const app = buildServer({ memory: memory(), consumerSecret: 'secret', forwarder: { forward } })
    const response = await app.inject({
      method: 'POST',
      url: '/webhooks/x',
      headers: { 'content-type': 'application/json', 'x-twitter-webhooks-signature': 'sha256=nope' },
      payload,
    })
    expect(response.statusCode).toBe(401)
    expect(forward).not.toHaveBeenCalled()
    await app.close()
  })

  it('answers X even when the receiver is slow or failing', async () => {
    let release: () => void = () => {}
    const forward = vi.fn().mockReturnValue(new Promise<void>((resolve) => { release = resolve }))
    const app = buildServer({ memory: memory(), consumerSecret: 'secret', forwarder: { forward } })
    const response = await app.inject({
      method: 'POST',
      url: '/webhooks/x',
      headers: { 'content-type': 'application/json', 'x-twitter-webhooks-signature': signature },
      payload,
    })
    expect(response.statusCode).toBe(200)
    release()
    const failing = vi.fn().mockRejectedValue(new Error('receiver down'))
    const app2 = buildServer({ memory: memory(), consumerSecret: 'secret', forwarder: { forward: failing } })
    const again = await app2.inject({
      method: 'POST',
      url: '/webhooks/x',
      headers: { 'content-type': 'application/json', 'x-twitter-webhooks-signature': signature },
      payload,
    })
    expect(again.statusCode).toBe(200)
    await app.close()
    await app2.close()
  })
})

describe('the forwarder', () => {
  it('posts the bytes with the signature and a bearer, and swallows a refusal', async () => {
    const calls: Array<{ url: string; init: RequestInit }> = []
    const fetchMock = vi.fn(async (url: string | URL | Request, init?: RequestInit) => {
      calls.push({ url: String(url), init: init ?? {} })
      return new Response('no', { status: 503 })
    })
    const warn = vi.fn()
    const { buildWebhookForwarder } = await import('../src/webhooks/forward.js')
    const forwarder = buildWebhookForwarder({
      url: 'https://backend.example/x',
      token: 'a-token-of-sixteen-chars',
      fetch: fetchMock as unknown as typeof fetch,
      logger: { info: vi.fn(), warn },
    })
    await expect(forwarder.forward(Buffer.from('{"a":1}'), 'sha256=abc')).resolves.toBeUndefined()
    expect(calls).toHaveLength(1)
    expect(calls[0]!.url).toBe('https://backend.example/x')
    expect(calls[0]!.init.method).toBe('POST')
    expect(calls[0]!.init.headers).toMatchObject({
      'content-type': 'application/json',
      'x-twitter-webhooks-signature': 'sha256=abc',
      authorization: 'Bearer a-token-of-sixteen-chars',
    })
    expect(Buffer.from(calls[0]!.init.body as Uint8Array).toString('utf8')).toBe('{"a":1}')
    expect(warn).toHaveBeenCalledWith(expect.objectContaining({ status: 503 }), 'webhook copy refused')
  })

  it('survives a receiver that throws', async () => {
    const { buildWebhookForwarder } = await import('../src/webhooks/forward.js')
    const warn = vi.fn()
    const forwarder = buildWebhookForwarder({
      url: 'https://backend.example/x',
      fetch: vi.fn().mockRejectedValue(new Error('ECONNREFUSED')) as unknown as typeof fetch,
      logger: { info: vi.fn(), warn },
    })
    await expect(forwarder.forward(Buffer.from('{}'), undefined)).resolves.toBeUndefined()
    expect(warn).toHaveBeenCalledWith(expect.objectContaining({ err: 'ECONNREFUSED' }), 'webhook copy not delivered')
  })
})

describe('X Activity API deliveries', () => {
  const BOT = '2094854245468000256'
  const options = { botUserId: BOT, botUsername: 'TinyMascot' }

  function event(eventType: string, payload: Record<string, unknown>, includes?: Record<string, unknown>) {
    return {
      data: {
        event_type: eventType,
        event_uuid: '777',
        filter: { user_id: BOT },
        tag: `teeny ${eventType}`,
        payload,
        ...(includes ? { includes } : {}),
      },
    }
  }

  it('reads a mention out of the envelope, with the author from includes', async () => {
    const { mentionsFromActivity } = await import('../src/webhooks/activity.js')
    const mentions = mentionsFromActivity(
      event(
        'post.mention.create',
        { id: '900', text: 'hey @TinyMascot', author_id: '42', created_at: '2026-09-14T09:40:00.000Z' },
        { users: [{ id: '42', username: 'someone' }] },
      ),
      options,
    )
    expect(mentions).toEqual([
      { id: '900', text: 'hey @TinyMascot', authorId: '42', authorUsername: 'someone', createdAt: '2026-09-14T09:40:00.000Z' },
    ])
  })

  it('takes a reply to the account as one, and keeps the reply target', async () => {
    const { mentionsFromActivity } = await import('../src/webhooks/activity.js')
    const mentions = mentionsFromActivity(
      event('post.reply.create', { id: '901', text: 'nice one', author_id: '42', in_reply_to_user_id: BOT }),
      options,
    )
    expect(mentions).toEqual([{ id: '901', text: 'nice one', authorId: '42', inReplyToUserId: BOT }])
  })

  it('answers neither the account itself nor an event that is not a post', async () => {
    const { mentionsFromActivity } = await import('../src/webhooks/activity.js')
    expect(
      mentionsFromActivity(event('post.mention.create', { id: '902', text: '@TinyMascot me', author_id: BOT }), options),
    ).toEqual([])
    expect(mentionsFromActivity(event('like.create', { id: '903', text: '@TinyMascot', author_id: '42' }), options)).toEqual([])
    expect(mentionsFromActivity(event('post.mention.create', {}), options)).toEqual([])
  })

  it('captures the other events under their event type, and no post events', async () => {
    const { activityEntries, captureWebhookActivity } = await import('../src/webhooks/activity.js')
    expect(activityEntries(event('follow.follow', { source: { id: '42' }, target: { id: BOT } }))).toEqual([
      { kind: 'follow.follow', id: '777', payload: { source: { id: '42' }, target: { id: BOT } } },
    ])
    const store = memory()
    await captureWebhookActivity(store, event('post.mention.create', { id: '904', text: '@TinyMascot', author_id: '42' }))
    expect(store.captureActivity).not.toHaveBeenCalled()
    await captureWebhookActivity(store, event('like.create', { id: '905', author_id: '42' }))
    expect(store.captureActivity).toHaveBeenCalledWith('like.create', '905', { id: '905', author_id: '42' })
  })

  it('queues a mention from the new envelope end to end', async () => {
    const payload = JSON.stringify(
      event('post.mention.create', { id: '906', text: 'yo @TinyMascot', author_id: '42' }, { users: [{ id: '42', username: 'someone' }] }),
    )
    const cache = vi.fn().mockResolvedValue(1)
    const app = buildServer({
      memory: memory(),
      consumerSecret: 'secret',
      queue: { cache, takePending: vi.fn(), markReplied: vi.fn(), markFailed: vi.fn(), requeueApproved: vi.fn(), markSkipped: vi.fn(), markLiked: vi.fn(), countReplied: vi.fn().mockResolvedValue(0), counts: vi.fn(), close: vi.fn() },
      botUserId: async () => BOT,
      botUsername: 'TinyMascot',
    })
    const response = await app.inject({
      method: 'POST',
      url: '/webhooks/x',
      headers: {
        'content-type': 'application/json',
        'x-twitter-webhooks-signature': `sha256=${createHmac('sha256', 'secret').update(payload).digest('base64')}`,
      },
      payload,
    })
    expect(response.statusCode).toBe(200)
    expect(cache).toHaveBeenCalledWith([{ id: '906', text: 'yo @TinyMascot', authorId: '42', authorUsername: 'someone' }])
    await app.close()
  })
})

describe('interactions from deliveries', () => {
  const BOT = '2094854245468000256'
  const event = (eventType: string, payload: Record<string, unknown>, includes?: Record<string, unknown>) => ({
    data: { event_type: eventType, event_uuid: '1', filter: { user_id: BOT }, payload, ...(includes ? { includes } : {}) },
  })

  it('credits the author, the follower, or the liker, never the account', async () => {
    const { interactionsFromActivity } = await import('../src/webhooks/activity.js')
    expect(interactionsFromActivity(event('post.quote.create', { id: '1', author_id: '42' }, { users: [{ id: '42', username: 'q' }] }), BOT)).toEqual([
      { userId: '42', username: 'q', kind: 'quote' },
    ])
    expect(interactionsFromActivity(event('post.repost.create', { id: '1', author_id: '43' }), BOT)).toEqual([{ userId: '43', kind: 'repost' }])
    expect(interactionsFromActivity(event('follow.follow', { source: { id: '44' }, target: { id: BOT } }), BOT)).toEqual([{ userId: '44', kind: 'follow' }])
    expect(interactionsFromActivity(event('like.create', { liked_tweet_id: '9', tweet_author_id: BOT }, { users: [{ id: BOT }, { id: '45', username: 'l' }] }), BOT)).toEqual([
      { userId: '45', username: 'l', kind: 'like' },
    ])
    expect(interactionsFromActivity(event('post.mention.create', { id: '1', author_id: BOT }), BOT)).toEqual([])
    expect(interactionsFromActivity(event('profile.update.bio', { before: 'a', after: 'b' }), BOT)).toEqual([])
    expect(interactionsFromActivity({ tweet_create_events: [] }, BOT)).toEqual([])
  })

  it('records them from the webhook', async () => {
    const payload = JSON.stringify(event('follow.follow', { source: { id: '44' }, target: { id: BOT } }))
    const observe = vi.fn().mockResolvedValue(undefined)
    const app = buildServer({
      memory: memory(),
      consumerSecret: 'secret',
      queue: { cache: vi.fn().mockResolvedValue(0), takePending: vi.fn(), markReplied: vi.fn(), markFailed: vi.fn(), requeueApproved: vi.fn(), markSkipped: vi.fn(), markLiked: vi.fn(), countReplied: vi.fn().mockResolvedValue(0), counts: vi.fn(), close: vi.fn() },
      botUserId: async () => BOT,
      botUsername: 'TinyMascot',
      people: { observe },
    })
    const response = await app.inject({
      method: 'POST',
      url: '/webhooks/x',
      headers: {
        'content-type': 'application/json',
        'x-twitter-webhooks-signature': `sha256=${createHmac('sha256', 'secret').update(payload).digest('base64')}`,
      },
      payload,
    })
    expect(response.statusCode).toBe(200)
    expect(observe).toHaveBeenCalledWith({ userId: '44', kind: 'follow' })
    await app.close()
  })
})
