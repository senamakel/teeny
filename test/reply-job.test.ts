import type { FastifyBaseLogger } from 'fastify'
import { describe, expect, it, vi } from 'vitest'
import { ReplyHarness } from '../src/harness/reply-harness.js'
import { IngestJob } from '../src/jobs/ingest-job.js'
import { ReplyJob } from '../src/jobs/reply-job.js'
import { MediaStore } from '../src/media/store.js'
import type { MentionQueue, QueuedMention } from '../src/queue/mentions.js'
import type {
  BotState,
  GifSource,
  LanguageModel,
  Memory,
  StateStore,
  XClient,
  XPost,
  XTools,
} from '../src/types.js'

const BOT_ID = '2094854245468000256'

const POSTS: XPost[] = [
  { id: '11', text: '@bot second' },
  { id: '10', text: '@bot first' },
]

function logger(): FastifyBaseLogger {
  return { info: vi.fn(), warn: vi.fn(), error: vi.fn() } as unknown as FastifyBaseLogger
}

function stateStore() {
  let saved: BotState = { repliedPostIds: [] }
  const store: StateStore = {
    load: vi.fn().mockImplementation(async () => structuredClone(saved)),
    save: vi.fn().mockImplementation(async (next: BotState) => {
      saved = structuredClone(next)
    }),
  }
  return { store, get: () => saved }
}

function queue(pending: XPost[] = []): MentionQueue {
  return {
    cache: vi.fn().mockResolvedValue(pending.length),
    takePending: vi.fn().mockResolvedValue(
      pending.map<QueuedMention>((post) => ({
        postId: post.id,
        post,
        status: 'pending',
        attempts: 0,
        cachedAt: '2026-09-02T00:00:00.000Z',
        updatedAt: '2026-09-02T00:00:00.000Z',
      })),
    ),
    markReplied: vi.fn().mockResolvedValue(undefined),
    markFailed: vi.fn().mockResolvedValue(undefined),
    counts: vi.fn(),
    close: vi.fn(),
  }
}

describe('IngestJob', () => {
  function subject(found: XPost[] = POSTS) {
    const x = { findMentions: vi.fn().mockResolvedValue(found), reply: vi.fn() } as XClient
    const state = stateStore()
    const mentions = queue()
    return {
      x,
      state,
      queue: mentions,
      job: new IngestJob({
        x,
        queue: mentions,
        state: state.store,
        logger: logger(),
        botUsername: 'bot',
      }),
    }
  }

  it('caches what it retrieved and advances the cursor past all of it', async () => {
    const it_ = subject()
    await it_.job.run()

    expect(it_.queue.cache).toHaveBeenCalledWith(POSTS)
    expect(it_.state.get().sinceId).toBe('11')
    expect(it_.state.get().lastPolledAt).toEqual(expect.any(String))
  })

  it('records the poll even when nothing matched, so the window stays narrow', async () => {
    const it_ = subject([])
    await it_.job.run()

    expect(it_.state.get().lastPolledAt).toEqual(expect.any(String))
    expect(it_.state.get().sinceId).toBeUndefined()
  })

  it('never sends since_id and start_time together, which X rejects with a 400', async () => {
    const it_ = subject()
    await it_.job.run()
    await it_.job.run()

    for (const [window] of (it_.x.findMentions as ReturnType<typeof vi.fn>).mock.calls) {
      expect(Boolean(window.sinceId) && Boolean(window.startTime)).toBe(false)
    }
    const second = (it_.x.findMentions as ReturnType<typeof vi.fn>).mock.calls[1]?.[0]
    expect(second).toMatchObject({ sinceId: '11' })
  })

  it('starts a fresh state at the configured lookback', async () => {
    const x = { findMentions: vi.fn().mockResolvedValue([]), reply: vi.fn() } as XClient
    const state = stateStore()
    await new IngestJob({
      x,
      queue: queue(),
      state: state.store,
      logger: logger(),
      lookbackMinutes: 30,
      botUsername: 'bot',
    }).run()

    const { startTime } = (x.findMentions as ReturnType<typeof vi.fn>).mock.calls[0]![0]
    const minutesBack = (Date.now() - Date.parse(startTime)) / 60_000
    expect(minutesBack).toBeGreaterThan(29)
    expect(minutesBack).toBeLessThan(31)
  })
})

describe('ReplyJob', () => {
  function subject(
    dryRun: boolean,
    options: { gifs?: GifSource; media?: MediaStore; draft?: unknown; pending?: XPost[] } = {},
  ) {
    const x = {
      findMentions: vi.fn(),
      reply: vi.fn(),
      createPost: vi.fn().mockResolvedValue({ id: '99' }),
      uploadMedia: vi.fn().mockResolvedValue({ mediaId: 'media-1' }),
    } as XClient & Pick<XTools, 'createPost' | 'uploadMedia'>
    const memory: Memory = {
      captureActivity: vi.fn().mockResolvedValue(undefined),
    captureMention: vi.fn().mockResolvedValue(undefined),
      recall: vi.fn().mockResolvedValue('context'),
      captureReply: vi.fn().mockResolvedValue(undefined),
    }
    const model: LanguageModel = {
      reply: vi.fn().mockResolvedValue(options.draft ?? { text: 'reply' }),
    }
    const mentions = queue(options.pending ?? [{ id: '10', text: '@bot first' }])

    return {
      x,
      memory,
      queue: mentions,
      job: new ReplyJob({
        x,
        harness: new ReplyHarness(memory, model),
        queue: mentions,
        logger: logger(),
        dryRun,
        maxReplies: 5,
        botUserId: async () => BOT_ID,
        ...(options.gifs ? { gifs: options.gifs } : {}),
        ...(options.media ? { media: options.media } : {}),
      }),
    }
  }

  it('replies to a cached mention and marks it replied', async () => {
    const it_ = subject(false)
    await it_.job.run()

    expect(it_.x.createPost).toHaveBeenCalledWith('reply', { replyToPostId: '10' })
    expect(it_.queue.markReplied).toHaveBeenCalledWith('10', { id: '99', text: 'reply' })
  })

  it('does not post in dry-run mode but still records the draft', async () => {
    const it_ = subject(true)
    await it_.job.run()

    expect(it_.x.createPost).not.toHaveBeenCalled()
    expect(it_.queue.markReplied).toHaveBeenCalledWith('10', { text: 'reply' })
  })

  it('leaves a failed mention marked for a later attempt', async () => {
    const it_ = subject(false)
    ;(it_.x.createPost as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('X is down'))
    await it_.job.run()

    expect(it_.queue.markFailed).toHaveBeenCalledWith('10', 'X is down')
    expect(it_.queue.markReplied).not.toHaveBeenCalled()
  })

  it('enforces the emoji cap the model ignored', async () => {
    const it_ = subject(false, { draft: { text: 'gm frens 👀 💛 🛠️' } })
    await it_.job.run()

    expect(it_.x.createPost).toHaveBeenCalledWith('gm frens 👀', { replyToPostId: '10' })
  })

  it('falls back beyond the channel when it has nothing', async () => {
    const channelGif = vi.fn()
    const gifs: GifSource = {
      search: channelGif
        .mockResolvedValueOnce([])
        .mockResolvedValueOnce([
          {
            id: 'g2',
            title: 'wider',
            pageUrl: 'https://giphy.com/gifs/g2',
            mediaUrl: 'https://media.giphy.com/g2.gif',
            mimeType: 'image/gif',
          },
        ]),
      get: vi.fn(),
      download: vi
        .fn()
        .mockResolvedValue({ data: new Uint8Array([1]), mimeType: 'image/gif' }),
    }
    const it_ = subject(false, { gifs, draft: { text: 'reply', gifQuery: 'peekaboo' } })
    await it_.job.run()

    expect(gifs.search).toHaveBeenNthCalledWith(1, 'peekaboo', { channelOnly: true })
    expect(gifs.search).toHaveBeenNthCalledWith(2, 'peekaboo', { channelOnly: false })
    expect(it_.x.createPost).toHaveBeenCalledWith('reply', {
      replyToPostId: '10',
      mediaIds: ['media-1'],
    })
  })

  it('attaches a GIF when the drafter asks for one', async () => {
    const gifs: GifSource = {
      search: vi.fn().mockResolvedValue([
        {
          id: 'g1',
          title: 'wave',
          pageUrl: 'https://giphy.com/gifs/g1',
          mediaUrl: 'https://media.giphy.com/g1.gif',
          mimeType: 'image/gif',
        },
      ]),
      get: vi.fn(),
      download: vi
        .fn()
        .mockResolvedValue({ data: new Uint8Array([1, 2, 3]), mimeType: 'image/gif' }),
    }
    const it_ = subject(false, { gifs, draft: { text: 'reply', gifQuery: 'waving' } })
    await it_.job.run()

    expect(gifs.search).toHaveBeenCalledWith('waving', { channelOnly: true })
    expect(it_.x.createPost).toHaveBeenCalledWith('reply', {
      replyToPostId: '10',
      mediaIds: ['media-1'],
    })
  })

  it('attaches a file the drafter generated, and prefers it to a GIF', async () => {
    const media = new MediaStore()
    const stored = media.put({ kind: 'image', prompt: 'p', data: new Uint8Array([7, 7]), mimeType: 'image/png' })
    const gifs: GifSource = { search: vi.fn(), get: vi.fn(), download: vi.fn() }
    const it_ = subject(false, {
      gifs,
      media,
      draft: { text: 'reply', gifQuery: 'waving', mediaId: stored.id },
    })
    await it_.job.run()

    expect(it_.x.uploadMedia).toHaveBeenCalledWith({ data: new Uint8Array([7, 7]), mimeType: 'image/png' })
    expect(gifs.search).not.toHaveBeenCalled()
    expect(it_.x.createPost).toHaveBeenCalledWith('reply', { replyToPostId: '10', mediaIds: ['media-1'] })
  })

  it('replies with text alone when the mediaId is not in the store or will not upload', async () => {
    const media = new MediaStore()
    const missing = subject(false, { media, draft: { text: 'reply', mediaId: 'img_nope_1' } })
    await missing.job.run()
    expect(missing.x.uploadMedia).not.toHaveBeenCalled()
    expect(missing.x.createPost).toHaveBeenCalledWith('reply', { replyToPostId: '10' })

    const stored = media.put({ kind: 'video', prompt: 'p', data: new Uint8Array([1]), mimeType: 'video/mp4' })
    const failing = subject(false, { media, draft: { text: 'reply', mediaId: stored.id } })
    ;(failing.x.uploadMedia as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('too big'))
    await failing.job.run()
    expect(failing.x.createPost).toHaveBeenCalledWith('reply', { replyToPostId: '10' })
  })

  it('still replies with text when the GIF cannot be fetched', async () => {
    const gifs: GifSource = {
      search: vi.fn().mockRejectedValue(new Error('giphy is down')),
      get: vi.fn(),
      download: vi.fn(),
    }
    const it_ = subject(false, { gifs, draft: { text: 'reply', gifQuery: 'waving' } })
    await it_.job.run()

    expect(it_.x.createPost).toHaveBeenCalledWith('reply', { replyToPostId: '10' })
  })
})

describe('ReplyJob self-reply guards', () => {
  function subject(
    pending: XPost[],
    botUserId: () => Promise<string | undefined>,
  ) {
    const x = {
      createPost: vi.fn().mockResolvedValue({ id: '99' }),
      reply: vi.fn().mockResolvedValue({ id: '99' }),
    } as unknown as XClient & XTools
    const mentions = queue(pending)
    const model: LanguageModel = {
      reply: vi.fn().mockResolvedValue({ text: 'reply', incomingTone: 'friendly' }),
    }
    const memory: Memory = {
      recall: vi.fn().mockResolvedValue(''),
      captureMention: vi.fn(),
      captureReply: vi.fn(),
      remember: vi.fn(),
    } as unknown as Memory
    return {
      x,
      queue: mentions,
      job: new ReplyJob({
        x,
        harness: new ReplyHarness(memory, model),
        queue: mentions,
        logger: logger(),
        dryRun: false,
        maxReplies: 5,
        botUserId,
      }),
    }
  }

  // The failure this guards against is not "one wrong reply". The account's
  // answer is itself activity on the account, so it comes back as the next
  // mention: answering it produces another, and so on.
  it('refuses a post the account itself wrote', async () => {
    const own: XPost = { id: '10', text: 'gm', authorId: BOT_ID }
    const it_ = subject([own], async () => BOT_ID)

    await it_.job.run()

    expect(it_.x.reply).not.toHaveBeenCalled()
    expect(it_.queue.markFailed).toHaveBeenCalledWith('10', expect.stringContaining('refused'))
  })

  it('refuses the whole run when the account cannot be identified', async () => {
    const it_ = subject([{ id: '10', text: '@bot hi', authorId: '1' }], async () => undefined)

    await it_.job.run()

    expect(it_.x.reply).not.toHaveBeenCalled()
    expect(it_.queue.markFailed).not.toHaveBeenCalled()
  })
})

describe('thread depth', () => {
  const BOT = '2094854245468000256'

  function subject(found: XPost[]) {
    const x = { findMentions: vi.fn().mockResolvedValue(found), reply: vi.fn() } as XClient
    const state = stateStore()
    const mentions = queue()
    return {
      state,
      queue: mentions,
      job: new IngestJob({
        x,
        queue: mentions,
        state: state.store,
        logger: logger(),
        botUserId: async () => BOT,
        botUsername: 'bot',
      }),
    }
  }

  it('queues a direct mention and a direct reply to the account', async () => {
    const posts: XPost[] = [
      { id: '20', text: '@bot hello', authorId: '1' },
      { id: '21', text: '@bot nice', authorId: '2', inReplyToUserId: BOT },
    ]
    const it_ = subject(posts)
    await it_.job.run()

    expect(it_.queue.cache).toHaveBeenCalledWith(posts)
  })

  it('drops a deep reply that only carries the handle X prefixed for it', async () => {
    const shallow: XPost = { id: '30', text: '@bot hi', authorId: '1' }
    const inherited: XPost = {
      id: '31',
      text: '@someone @bot Rhyming is a bold choice for the timeline',
      authorId: '2',
      inReplyToUserId: '999',
    }
    const it_ = subject([shallow, inherited])
    await it_.job.run()

    expect(it_.queue.cache).toHaveBeenCalledWith([shallow])
  })

  it('keeps a deep reply where somebody actually typed the handle', async () => {
    const called: XPost = {
      id: '32',
      text: '@someone Oh hi fren @bot',
      authorId: '2',
      inReplyToUserId: '999',
    }
    const it_ = subject([called])
    await it_.job.run()

    expect(it_.queue.cache).toHaveBeenCalledWith([called])
  })

  it('still advances the cursor past what it dropped, so it is not re-read', async () => {
    const it_ = subject([
      { id: '40', text: '@someone @bot deep', authorId: '2', inReplyToUserId: '999' },
      { id: '41', text: '@someone @bot deeper', authorId: '3', inReplyToUserId: '888' },
    ])
    await it_.job.run()

    expect(it_.queue.cache).toHaveBeenCalledWith([])
    expect(it_.state.get().sinceId).toBe('41')
  })
})

describe('pacing', () => {
  const BOT = '2094854245468000256'

  function subject(minReplyIntervalMs: number, pending: XPost[]) {
    const x = {
      createPost: vi.fn().mockResolvedValue({ id: '99' }),
      reply: vi.fn().mockResolvedValue({ id: '99' }),
    } as unknown as XClient & XTools
    const memory = {
      recall: vi.fn().mockResolvedValue(''),
      captureMention: vi.fn(),
      captureReply: vi.fn(),
    } as unknown as Memory
    const model: LanguageModel = {
      reply: vi.fn().mockResolvedValue({ text: 'hi', incomingTone: 'friendly' }),
    }
    return {
      x,
      job: new ReplyJob({
        x,
        harness: new ReplyHarness(memory, model),
        queue: queue(pending),
        logger: logger(),
        dryRun: false,
        maxReplies: 5,
        botUserId: async () => BOT,
        minReplyIntervalMs,
      }),
    }
  }

  // A batch of five used to leave five posts back to back, which is what X
  // read as spam. The brake belongs at the publish site so the batch size
  // cannot reintroduce the burst.
  it('holds the second reply of a batch back', async () => {
    const posts: XPost[] = [
      { id: '1', text: '@bot one', authorId: 'a' },
      { id: '2', text: '@bot two', authorId: 'b' },
    ]
    const it_ = subject(80, posts)

    const started = Date.now()
    await it_.job.run()

    expect(it_.x.createPost).toHaveBeenCalledTimes(2)
    expect(Date.now() - started).toBeGreaterThanOrEqual(70)
  })

  it('does not delay the first reply', async () => {
    const it_ = subject(10_000, [{ id: '1', text: '@bot one', authorId: 'a' }])

    const started = Date.now()
    await it_.job.run()

    expect(it_.x.createPost).toHaveBeenCalledTimes(1)
    expect(Date.now() - started).toBeLessThan(1_000)
  })

  it('keeps the gap across runs, not just within a batch', async () => {
    const it_ = subject(80, [{ id: '1', text: '@bot one', authorId: 'a' }])
    await it_.job.run()

    const started = Date.now()
    await it_.job.run()

    expect(Date.now() - started).toBeGreaterThanOrEqual(70)
  })

  it('does not pace a dry run, which publishes nothing', async () => {
    const x = {
      createPost: vi.fn(),
      reply: vi.fn(),
    } as unknown as XClient & XTools
    const memory = {
      recall: vi.fn().mockResolvedValue(''),
      captureMention: vi.fn(),
      captureReply: vi.fn(),
    } as unknown as Memory
    const job = new ReplyJob({
      x,
      harness: new ReplyHarness(memory, {
        reply: vi.fn().mockResolvedValue({ text: 'hi', incomingTone: 'friendly' }),
      } as unknown as LanguageModel),
      queue: queue([
        { id: '1', text: '@bot one', authorId: 'a' },
        { id: '2', text: '@bot two', authorId: 'b' },
      ]),
      logger: logger(),
      dryRun: true,
      maxReplies: 5,
      botUserId: async () => BOT,
      minReplyIntervalMs: 10_000,
    })

    const started = Date.now()
    await job.run()

    expect(Date.now() - started).toBeLessThan(1_000)
  })
})

describe('the gap is random, not a metronome', () => {
  const BOT = '2094854245468000256'

  function paced(random: () => number, min: number, max: number) {
    const x = {
      createPost: vi.fn().mockResolvedValue({ id: '99' }),
      reply: vi.fn().mockResolvedValue({ id: '99' }),
    } as unknown as XClient & XTools
    const memory = {
      recall: vi.fn().mockResolvedValue(''),
      captureMention: vi.fn(),
      captureReply: vi.fn(),
    } as unknown as Memory
    const log = logger()
    return {
      log,
      job: new ReplyJob({
        x,
        harness: new ReplyHarness(memory, {
          reply: vi.fn().mockResolvedValue({ text: 'hi', incomingTone: 'friendly' }),
        } as unknown as LanguageModel),
        queue: queue([
          { id: '1', text: '@bot one', authorId: 'a' },
          { id: '2', text: '@bot two', authorId: 'b' },
        ]),
        logger: log,
        dryRun: false,
        maxReplies: 5,
        botUserId: async () => BOT,
        minReplyIntervalMs: min,
        maxReplyIntervalMs: max,
        random,
      }),
    }
  }

  function chosenGaps(log: ReturnType<typeof logger>) {
    return (log.info as unknown as { mock: { calls: unknown[][] } }).mock.calls
      .map(([first]) => first as { gap?: number })
      .filter((entry) => typeof entry?.gap === 'number')
      .map((entry) => entry.gap!)
  }

  it('takes the bottom of the range when the roll is low', async () => {
    const it_ = paced(() => 0, 20, 100)
    await it_.job.run()

    expect(chosenGaps(it_.log)).toEqual([20])
  })

  it('takes the top of the range when the roll is high', async () => {
    const it_ = paced(() => 1, 20, 100)
    await it_.job.run()

    expect(chosenGaps(it_.log)).toEqual([100])
  })

  it('lands in between for anything else', async () => {
    const it_ = paced(() => 0.5, 20, 120)
    await it_.job.run()

    expect(chosenGaps(it_.log)).toEqual([70])
  })

  // A range that is backwards, or a maximum below the minimum, must not
  // produce a negative wait and quietly disable the brake.
  it('never goes below the minimum, whatever the range says', async () => {
    const it_ = paced(() => 0.9, 50, 10)
    await it_.job.run()

    expect(chosenGaps(it_.log)).toEqual([50])
  })
})
