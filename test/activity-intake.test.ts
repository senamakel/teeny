import { describe, expect, it, vi } from 'vitest'
import { ActivityIntake } from '../src/webhooks/intake.js'
import type { Memory } from '../src/types.js'
import type { MentionQueue } from '../src/queue/mentions.js'

const BOT = '2094854245468000256'

function memory(): Memory {
  return {
    captureActivity: vi.fn().mockResolvedValue(undefined),
    captureMention: vi.fn().mockResolvedValue(undefined),
    recall: vi.fn().mockResolvedValue(''),
    captureReply: vi.fn().mockResolvedValue(undefined),
  }
}

function logger() {
  return { debug: vi.fn(), info: vi.fn(), warn: vi.fn(), error: vi.fn() } as never
}

function queue() {
  return { cache: vi.fn().mockResolvedValue(1) } as unknown as MentionQueue & { cache: ReturnType<typeof vi.fn> }
}

function mention(id: string, eventId = id) {
  return {
    data: {
      event_uuid: eventId,
      event_type: 'post.mention.create',
      filter: { user_id: BOT },
      payload: { id, text: 'hey @teenydroid', author_id: '42', created_at: '2026-09-21T12:00:00.000Z' },
      includes: { users: [{ id: '42', username: 'someone' }] },
    },
  }
}

function like(eventId: string) {
  return {
    data: {
      event_uuid: eventId,
      event_type: 'like.create',
      filter: { user_id: BOT },
      payload: { id: 'l1', liked_tweet_id: '1', liking_user_id: '42' },
      includes: { users: [{ id: '42', username: 'someone' }] },
    },
  }
}

describe('activity intake', () => {
  it('queues a streamed mention and scores the author', async () => {
    const mentions = queue()
    const people = { observe: vi.fn().mockResolvedValue(undefined) }
    const intake = new ActivityIntake({
      memory: memory(),
      logger: logger(),
      queue: mentions,
      botUserId: async () => BOT,
      botUsername: 'teenydroid',
      people,
    })

    expect(await intake.deliver(mention('900'), 'stream')).toEqual({ queued: 1, duplicate: false })
    expect(mentions.cache).toHaveBeenCalledWith([
      expect.objectContaining({ id: '900', authorId: '42', authorUsername: 'someone' }),
    ])
    expect(people.observe).toHaveBeenCalledWith({ userId: '42', username: 'someone', kind: 'mention' })
  })

  it('handles an event once when both transports deliver it', async () => {
    const mentions = queue()
    const people = { observe: vi.fn().mockResolvedValue(undefined) }
    const store = memory()
    const intake = new ActivityIntake({
      memory: store,
      logger: logger(),
      queue: mentions,
      botUserId: async () => BOT,
      botUsername: 'teenydroid',
      people,
    })

    await intake.deliver(like('e1'), 'stream')
    expect(await intake.deliver(like('e1'), 'webhook')).toEqual({ queued: 0, duplicate: true })
    await intake.deliver(like('e2'), 'webhook')

    // A like counts once per event, not once per transport.
    expect(people.observe).toHaveBeenCalledTimes(2)
    await vi.waitFor(() => expect(store.captureActivity).toHaveBeenCalledTimes(2))
  })

  it('forgets the oldest ids once the set is full', async () => {
    const people = { observe: vi.fn().mockResolvedValue(undefined) }
    const intake = new ActivityIntake({
      memory: memory(),
      logger: logger(),
      queue: queue(),
      botUserId: async () => BOT,
      botUsername: 'teenydroid',
      people,
      remember: 2,
    })

    await intake.deliver(like('e1'), 'stream')
    await intake.deliver(like('e2'), 'stream')
    await intake.deliver(like('e3'), 'stream')
    expect((await intake.deliver(like('e1'), 'webhook')).duplicate).toBe(false)
    expect((await intake.deliver(like('e3'), 'webhook')).duplicate).toBe(true)
  })

  it('answers before memory has the event, and survives memory failing', async () => {
    const store = memory()
    let release!: () => void
    ;(store.captureActivity as ReturnType<typeof vi.fn>).mockReturnValue(
      new Promise<void>((resolve) => {
        release = resolve
      }),
    )
    const intake = new ActivityIntake({ memory: store, logger: logger() })

    const done = await Promise.race([
      intake.deliver(like('e1'), 'webhook').then(() => 'delivered'),
      new Promise((resolve) => setTimeout(() => resolve('stuck'), 200)),
    ])
    expect(done).toBe('delivered')
    release()

    ;(store.captureActivity as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('cortex timed out'))
    await expect(intake.deliver(like('e2'), 'webhook')).resolves.toEqual({ queued: 0, duplicate: false })
  })
})
