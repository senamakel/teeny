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
      counts: vi.fn(),
      close: vi.fn(),
    }
  }

  async function post(payload: string, mentions: ReturnType<typeof queue>) {
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
