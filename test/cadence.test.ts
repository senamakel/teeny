import type { FastifyBaseLogger } from 'fastify'
import { describe, expect, it, vi } from 'vitest'
import { heldSlots, ownPostOf, ReplyBudget, type Lane } from '../src/cadence/budget.js'
import { Cadence, CadenceHeld, paced } from '../src/cadence/cadence.js'
import { baselineOf, HeatJob, velocityOf } from '../src/cadence/heat.js'
import { InMemoryPostLedger } from '../src/cadence/ledger.js'
import { InMemoryPeople, WEIGHTS } from '../src/cadence/people.js'
import { ReplyHarness } from '../src/harness/reply-harness.js'
import { ReplyJob } from '../src/jobs/reply-job.js'
import type { MentionQueue, QueuedMention } from '../src/queue/mentions.js'
import type { LanguageModel, Memory, XClient, XPost, XTools } from '../src/types.js'

const HOUR = 3_600_000
const T0 = Date.parse('2026-09-14T07:00:00.000Z')
const iso = (ms: number) => new Date(ms).toISOString()
const BOT = '2094854245468000256'

function logger(): FastifyBaseLogger {
  return { info: vi.fn(), warn: vi.fn(), error: vi.fn() } as unknown as FastifyBaseLogger
}

describe('the cadence', () => {
  function subject(now: number) {
    const ledger = new InMemoryPostLedger()
    const clock = { now }
    const cadence = new Cadence({ ledger, minGapMs: 8 * HOUR, toleranceMs: 20 * 60_000, now: () => clock.now })
    return { ledger, cadence, clock }
  }

  it('lets the first post through, then refuses one inside the gap', async () => {
    const { cadence, clock } = subject(T0)
    expect(await cadence.claim('morning')).toEqual({ ok: true })
    await cadence.record({ postId: '1', kind: 'morning' })
    clock.now = T0 + 5 * HOUR
    const claim = await cadence.claim('chart')
    expect(claim).toMatchObject({ ok: false, reason: 'gap', lastPostId: '1', until: iso(T0 + 8 * HOUR) })
  })

  it('counts the tolerance as the gap, so a slot eight hours later that ran late still posts', async () => {
    const { cadence, clock } = subject(T0)
    await cadence.record({ postId: '1', kind: 'morning' })
    clock.now = T0 + 8 * HOUR - 15 * 60_000
    expect(await cadence.claim('chart')).toEqual({ ok: true })
    clock.now = T0 + 8 * HOUR - 25 * 60_000
    expect((await cadence.claim('chart')).ok).toBe(false)
  })

  it('holds the timeline for a hot post, whatever the gap says', async () => {
    const { cadence, ledger, clock } = subject(T0)
    await cadence.record({ postId: '1', kind: 'morning' })
    await ledger.setHot('1', { since: iso(T0 + HOUR), holdUntil: iso(T0 + 9 * HOUR) })
    clock.now = T0 + 8.5 * HOUR
    expect(await cadence.claim('chart')).toMatchObject({ ok: false, reason: 'hot', hotPostId: '1', until: iso(T0 + 9 * HOUR) })
    clock.now = T0 + 9.5 * HOUR
    expect(await cadence.claim('chart')).toEqual({ ok: true })
  })

  it('wraps the posting client: own posts are claimed and recorded, replies pass', async () => {
    const { cadence, ledger, clock } = subject(T0)
    const raw = {
      createPost: vi.fn().mockResolvedValue({ id: '77' }),
      uploadMedia: vi.fn(),
    } as unknown as XTools
    const x = paced(raw, cadence)
    await cadence.within('moon', () => x.createPost('to the moon', { mediaIds: ['m'] }))
    expect((await ledger.get('77'))?.kind).toBe('moon')
    clock.now = T0 + HOUR
    await expect(x.createPost('again')).rejects.toBeInstanceOf(CadenceHeld)
    await expect(x.createPost('a reply', { replyToPostId: '5' })).resolves.toEqual({ id: '77' })
    expect(raw.createPost).toHaveBeenCalledTimes(2)
    expect(ledger.posts.size).toBe(1)
  })

  it('wraps an article the same way: claimed, published, recorded as an own post', async () => {
    const { cadence, ledger, clock } = subject(T0)
    const raw = {
      createPost: vi.fn(),
      publishArticle: vi.fn().mockResolvedValue({ articleId: 'a1', postId: '88' }),
    } as unknown as XTools
    const x = paced(raw, cadence)
    const draft = { title: 'the inch', contentState: { blocks: [], entities: [] } }
    await expect(cadence.within('article', () => x.publishArticle(draft))).resolves.toEqual({ articleId: 'a1', postId: '88' })
    expect(raw.publishArticle).toHaveBeenCalledWith(draft)
    expect((await ledger.get('88'))?.kind).toBe('article')
    clock.now = T0 + HOUR
    await expect(x.publishArticle(draft)).rejects.toBeInstanceOf(CadenceHeld)
    expect(raw.publishArticle).toHaveBeenCalledTimes(1)
  })
})

describe('the heat job', () => {
  it('reads a velocity from two readings, and a baseline from the other posts', () => {
    const post = { postId: '1', kind: 'chart', postedAt: iso(T0), samples: [{ at: iso(T0 + HOUR), impressions: 100, likes: 0, replies: 0, reposts: 0, quotes: 0 }] }
    expect(velocityOf(post, { at: iso(T0 + 2 * HOUR), impressions: 700, likes: 0, replies: 0, reposts: 0, quotes: 0 })).toBe(600)
    expect(velocityOf(post, { at: iso(T0 + HOUR), impressions: 700, likes: 0, replies: 0, reposts: 0, quotes: 0 })).toBeUndefined()
    const others = [
      { postId: '2', kind: 'morning', postedAt: iso(T0), samples: [{ at: iso(T0 + HOUR), impressions: 50, likes: 0, replies: 0, reposts: 0, quotes: 0 }] },
      { postId: '3', kind: 'morning', postedAt: iso(T0), samples: [{ at: iso(T0 + HOUR), impressions: 150, likes: 0, replies: 0, reposts: 0, quotes: 0 }] },
      { postId: '4', kind: 'morning', postedAt: iso(T0), samples: [] },
    ]
    expect(baselineOf(others)).toBe(100)
    expect(baselineOf(others.slice(0, 1))).toBeUndefined()
  })

  it('holds one slot for a hot post and extends it while it stays hot, up to the cap', async () => {
    const ledger = new InMemoryPostLedger()
    await ledger.record({ postId: '1', kind: 'chart', postedAt: iso(T0) })
    await ledger.record({ postId: '2', kind: 'morning', postedAt: iso(T0 - 20 * HOUR) })
    await ledger.sample('2', { at: iso(T0 - 19 * HOUR), impressions: 100, likes: 0, replies: 0, reposts: 0, quotes: 0 })
    await ledger.record({ postId: '3', kind: 'moon', postedAt: iso(T0 - 22 * HOUR) })
    await ledger.sample('3', { at: iso(T0 - 21 * HOUR), impressions: 120, likes: 0, replies: 0, reposts: 0, quotes: 0 })
    const impressions = { '1': 0 }
    const clock = { now: T0 }
    const x = {
      postMetrics: vi.fn(async (ids: string[]) =>
        ids.map((id) => ({ id, impressions: id === '1' ? impressions['1'] : 100, likes: 0, replies: 0, reposts: 0, quotes: 0 })),
      ),
    }
    const heat = new HeatJob({
      ledger,
      x,
      lookbackMs: 24 * HOUR,
      gapMs: 8 * HOUR,
      impressionsPerHour: 500,
      baselineMultiple: 3,
      maxHoldSlots: 2,
      now: () => clock.now,
      logger: logger(),
    })

    // A first reading of nothing: not hot.
    clock.now = T0 + 20 * 60_000
    impressions['1'] = 50
    let readings = await heat.run()
    expect(readings.find((entry) => entry.postId === '1')?.hot).toBe(false)

    // 2000 impressions in twenty minutes: 6000/h, past 500/h and past 3x the 110/h baseline.
    clock.now = T0 + 40 * 60_000
    impressions['1'] = 2050
    readings = await heat.run()
    const hot = readings.find((entry) => entry.postId === '1')
    expect(hot).toMatchObject({ hot: true, velocity: 6000, baseline: 110, holdUntil: iso(T0 + 40 * 60_000 + 8 * HOUR) })
    expect((await ledger.get('1'))?.hot?.since).toBe(iso(T0 + 40 * 60_000))

    // Still hot an hour on: the hold moves out with it.
    clock.now = T0 + 100 * 60_000
    impressions['1'] = 4050
    readings = await heat.run()
    expect(readings.find((entry) => entry.postId === '1')?.holdUntil).toBe(iso(T0 + 100 * 60_000 + 8 * HOUR))

    // Nine hours on and still hot: capped at two slots from when it first ran hot.
    clock.now = T0 + 40 * 60_000 + 9 * HOUR
    impressions['1'] = 50_000
    readings = await heat.run()
    expect(readings.find((entry) => entry.postId === '1')?.holdUntil).toBe(iso(T0 + 40 * 60_000 + 16 * HOUR))

    // Cooled off: the hold stands where it was and is not extended further.
    clock.now = T0 + 40 * 60_000 + 10 * HOUR
    impressions['1'] = 50_010
    readings = await heat.run()
    expect(readings.find((entry) => entry.postId === '1')).toMatchObject({ hot: false })
    expect((await ledger.get('1'))?.hot?.holdUntil).toBe(iso(T0 + 40 * 60_000 + 16 * HOUR))
  })
})

describe('the reply budget', () => {
  function subject(replied: Array<{ lane: Lane; ownPostId?: string; at: string }>, now = T0 + 2 * HOUR) {
    const ledger = new InMemoryPostLedger()
    const queue = {
      countReplied: vi.fn(async (filter: { lane: Lane; ownPostId?: string; since?: string }) =>
        replied.filter(
          (entry) =>
            entry.lane === filter.lane &&
            (!filter.ownPostId || entry.ownPostId === filter.ownPostId) &&
            (!filter.since || entry.at >= filter.since),
        ).length,
      ),
    }
    const budget = new ReplyBudget({ ledger, queue, autoPerPost: 10, smartPerPost: 10, gapMs: 8 * HOUR, botUsername: 'bot', now: () => now })
    return { ledger, budget }
  }

  it('finds the own post a reply sits under, by conversation or by falling back to the latest', () => {
    const own = [
      { postId: '2', kind: 'chart', postedAt: iso(T0 + HOUR), samples: [] },
      { postId: '1', kind: 'morning', postedAt: iso(T0), samples: [] },
    ]
    expect(ownPostOf({ id: 'a', text: 'x', conversationId: '1', inReplyToUserId: BOT }, BOT, own)?.postId).toBe('1')
    expect(ownPostOf({ id: 'a', text: 'x', inReplyToUserId: BOT }, BOT, own)?.postId).toBe('2')
    expect(ownPostOf({ id: 'a', text: '@bot hi' }, BOT, own)).toBeUndefined()
    expect(ownPostOf({ id: 'a', text: 'x', conversationId: 'theirs', inReplyToUserId: 'someone' }, BOT, own)).toBeUndefined()
  })

  it('caps the auto lane at ten under each post', async () => {
    const used = Array.from({ length: 10 }, () => ({ lane: 'auto' as const, ownPostId: '1', at: iso(T0 + HOUR) }))
    const { ledger, budget } = subject(used)
    await ledger.record({ postId: '1', kind: 'morning', postedAt: iso(T0) })
    await ledger.record({ postId: '2', kind: 'chart', postedAt: iso(T0 + HOUR) })
    expect(await budget.assess({ id: 'a', text: 'nice', conversationId: '1', inReplyToUserId: BOT }, BOT)).toEqual({
      lane: 'auto', ownPostId: '1', allowed: false, used: 10, cap: 10,
    })
    expect(await budget.assess({ id: 'b', text: 'nice', conversationId: '2', inReplyToUserId: BOT }, BOT)).toMatchObject({
      lane: 'auto', ownPostId: '2', allowed: true, used: 0, cap: 10,
    })
  })

  it('gives a hot post the held slots’ allowance on top of its own', async () => {
    const { ledger, budget } = subject([])
    await ledger.record({ postId: '1', kind: 'morning', postedAt: iso(T0) })
    await ledger.setHot('1', { since: iso(T0 + HOUR), holdUntil: iso(T0 + HOUR + 16 * HOUR) })
    expect(heldSlots((await ledger.get('1'))!, 8 * HOUR)).toBe(2)
    expect(await budget.assess({ id: 'a', text: 'wow', conversationId: '1', inReplyToUserId: BOT }, BOT)).toMatchObject({
      lane: 'auto', cap: 10 + 2 * 20,
    })
  })

  it('caps the smart lane per own post published in the last day, one post’s worth at least', async () => {
    const used = Array.from({ length: 10 }, () => ({ lane: 'smart' as const, at: iso(T0 + HOUR) }))
    const { ledger, budget } = subject(used)
    // A quote, or a reply in somebody else's thread where the handle was
    // carried along rather than typed: the reply guy's lane.
    const quote = { id: 'a', text: 'lol look at this' }
    expect(await budget.assess(quote, BOT)).toEqual({ lane: 'smart', allowed: false, used: 10, cap: 10 })
    await ledger.record({ postId: '1', kind: 'morning', postedAt: iso(T0) })
    await ledger.record({ postId: '2', kind: 'chart', postedAt: iso(T0 + HOUR) })
    await ledger.record({ postId: '0', kind: 'moon', postedAt: iso(T0 - 30 * HOUR) })
    expect(await budget.assess(quote, BOT)).toEqual({ lane: 'smart', allowed: true, used: 10, cap: 20 })
  })

  it('always answers a post that typed the handle on purpose, whatever the day’s budget says', async () => {
    const spent = [
      ...Array.from({ length: 10 }, () => ({ lane: 'smart' as const, at: iso(T0 + HOUR) })),
      ...Array.from({ length: 40 }, () => ({ lane: 'called' as const, at: iso(T0 + HOUR) })),
    ]
    const { budget } = subject(spent)
    expect(await budget.assess({ id: 'a', text: 'hey @bot what do you make of this' }, BOT)).toEqual({
      lane: 'called', allowed: true, used: 40, cap: Number.POSITIVE_INFINITY,
    })
    // Deep in somebody else's thread, still called in by name.
    expect(await budget.assess({ id: 'b', text: '@someone @other come on @bot settle this', conversationId: 'theirs', inReplyToUserId: 'someone' }, BOT)).toMatchObject({
      lane: 'called', allowed: true,
    })
    // The handle carried along by X at the front of a reply is not a call.
    expect(await budget.assess({ id: 'c', text: '@someone @bot rhyming is a bold choice', conversationId: 'theirs', inReplyToUserId: 'someone' }, BOT)).toMatchObject({
      lane: 'smart', allowed: false,
    })
  })

  it('keeps a comment under the account’s own post in the auto lane even when it names the account', async () => {
    const used = Array.from({ length: 10 }, () => ({ lane: 'auto' as const, ownPostId: '1', at: iso(T0 + HOUR) }))
    const { ledger, budget } = subject(used)
    await ledger.record({ postId: '1', kind: 'morning', postedAt: iso(T0) })
    expect(await budget.assess({ id: 'a', text: '@bot hey @bot are you awake', conversationId: '1', inReplyToUserId: BOT }, BOT)).toEqual({
      lane: 'auto', ownPostId: '1', allowed: false, used: 10, cap: 10,
    })
  })
})

describe('the people record', () => {
  it('scores by what people did, and ranks the regulars first', async () => {
    const people = new InMemoryPeople()
    await people.observe({ userId: 'a', username: 'alice', kind: 'like' })
    await people.observe({ userId: 'a', kind: 'reply' })
    await people.observe({ userId: 'b', kind: 'quote' })
    await people.observe({ userId: 'c', kind: 'follow' })
    expect(await people.scores(['a', 'b', 'c', 'nobody'])).toEqual(
      new Map([['a', WEIGHTS.like + WEIGHTS.reply], ['b', WEIGHTS.quote], ['c', WEIGHTS.follow], ['nobody', 0]]),
    )
    const top = await people.top(2)
    expect(top.map((person) => person.userId)).toEqual(['a', 'b'])
    expect(top[0]).toMatchObject({ username: 'alice', counts: { like: 1, reply: 1 } })
  })
})

describe('the reply job with a budget', () => {
  function queued(posts: XPost[]): MentionQueue {
    return {
      cache: vi.fn(),
      takePending: vi.fn().mockResolvedValue(
        posts.map<QueuedMention>((post, index) => ({
          postId: post.id,
          post,
          status: 'pending',
          attempts: 0,
          cachedAt: iso(T0 + index * 60_000),
          updatedAt: iso(T0 + index * 60_000),
        })),
      ),
      markReplied: vi.fn().mockResolvedValue(undefined),
      markFailed: vi.fn().mockResolvedValue(undefined),
      requeueApproved: vi.fn().mockResolvedValue(true),
      markSkipped: vi.fn().mockResolvedValue(undefined),
      markLiked: vi.fn().mockResolvedValue(undefined),
      countReplied: vi.fn().mockResolvedValue(0),
      counts: vi.fn(),
      close: vi.fn(),
    }
  }

  function subject(options: {
    pending: XPost[]
    assess?: (post: XPost) => { lane: 'auto' | 'smart'; ownPostId?: string; allowed: boolean; used: number; cap: number }
    people?: InMemoryPeople
    maxReplies?: number
    dryRun?: boolean
  }) {
    const x = {
      findMentions: vi.fn(),
      reply: vi.fn(),
      createPost: vi.fn().mockResolvedValue({ id: '99' }),
      uploadMedia: vi.fn(),
      likePost: vi.fn().mockResolvedValue(undefined),
    } as unknown as XClient & Pick<XTools, 'createPost' | 'uploadMedia' | 'likePost'>
    const memory: Memory = {
      captureActivity: vi.fn().mockResolvedValue(undefined),
      captureMention: vi.fn().mockResolvedValue(undefined),
      recall: vi.fn().mockResolvedValue(''),
      captureReply: vi.fn().mockResolvedValue(undefined),
    }
    const model: LanguageModel = { reply: vi.fn().mockResolvedValue({ text: 'reply' }) }
    const queue = queued(options.pending)
    const job = new ReplyJob({
      x,
      harness: new ReplyHarness(memory, model),
      queue,
      logger: logger(),
      dryRun: options.dryRun ?? false,
      maxReplies: options.maxReplies ?? 5,
      botUserId: async () => BOT,
      ...(options.assess ? { budget: { assess: async (post: XPost) => options.assess!(post) } } : {}),
      ...(options.people ? { people: options.people } : {}),
    })
    return { x, queue, job, model }
  }

  it('likes every mention it takes, answers the ones in budget, sets the rest aside', async () => {
    const it_ = subject({
      pending: [
        { id: '10', text: '@bot one', authorId: 'a' },
        { id: '11', text: '@bot two', authorId: 'b' },
      ],
      assess: (post) => (post.id === '11' ? { lane: 'smart', allowed: false, used: 10, cap: 10 } : { lane: 'auto', ownPostId: '1', allowed: true, used: 3, cap: 10 }),
    })
    await it_.job.run()
    expect(it_.x.likePost).toHaveBeenCalledWith('10')
    expect(it_.x.likePost).toHaveBeenCalledWith('11')
    expect(it_.queue.markLiked).toHaveBeenCalledTimes(2)
    expect(it_.x.createPost).toHaveBeenCalledTimes(1)
    expect(it_.queue.markReplied).toHaveBeenCalledWith('10', { id: '99', text: 'reply', lane: 'auto', ownPostId: '1' })
    expect(it_.queue.markSkipped).toHaveBeenCalledWith('11', 'budget: smart lane at 10/10')
    expect(it_.model.reply).toHaveBeenCalledTimes(1)
  })

  it('does not like in a dry run, and does not like twice', async () => {
    const it_ = subject({ pending: [{ id: '10', text: '@bot one' }], dryRun: true })
    await it_.job.run()
    expect(it_.x.likePost).not.toHaveBeenCalled()
    const again = subject({ pending: [{ id: '10', text: '@bot one' }] })
    ;(again.queue.takePending as ReturnType<typeof vi.fn>).mockResolvedValue([
      { postId: '10', post: { id: '10', text: '@bot one' }, status: 'pending', attempts: 0, cachedAt: iso(T0), updatedAt: iso(T0), likedAt: iso(T0) },
    ])
    await again.job.run()
    expect(again.x.likePost).not.toHaveBeenCalled()
  })

  it('answers the regulars first when there is a backlog', async () => {
    const people = new InMemoryPeople()
    await people.observe({ userId: 'regular', kind: 'quote' })
    await people.observe({ userId: 'regular', kind: 'like' })
    await people.observe({ userId: 'occasional', kind: 'like' })
    const it_ = subject({
      pending: [
        { id: '10', text: '@bot first in', authorId: 'stranger' },
        { id: '11', text: '@bot second', authorId: 'occasional' },
        { id: '12', text: '@bot third', authorId: 'regular' },
        { id: '13', text: '@bot fourth', authorId: 'stranger' },
      ],
      people,
      maxReplies: 2,
    })
    await it_.job.run()
    expect(it_.queue.takePending).toHaveBeenCalledWith(10)
    const answered = (it_.x.createPost as ReturnType<typeof vi.fn>).mock.calls.map((call) => (call[1] as { replyToPostId: string }).replyToPostId)
    expect(answered).toEqual(['12', '11'])
    expect(people.people.get('regular')?.counts.answered).toBe(1)
  })
})
