import { describe, expect, it, vi } from 'vitest'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { Cadence } from '../src/cadence/cadence.js'
import type { Critic } from '../src/critic/algo.js'
import { NarrativeJob } from '../src/jobs/narrative-job.js'
import { OpenAITextModel, type TextModel } from '../src/media/reference.js'
import { MediaBudget, MediaStore } from '../src/media/store.js'
import type { ImageGenerator, ImageReader, VideoGenerator } from '../src/media/types.js'
import { NarrativePipeline, narrativeStyle, swapVisitor, type NarrativePipelineOptions, type NarrativeResult } from '../src/narrative/pipeline.js'
import { LikenessFinder, withLikeness } from '../src/narrative/likeness.js'
import { clean, runWritersRoom } from '../src/narrative/room.js'
import { producerTurnPrompt, renderTranscript, teenyTurnPrompt, type RoomContext } from '../src/narrative/stages.js'
import { FileNarrativeState, MemoryNarrativeState, type NarrativeState } from '../src/narrative/state.js'
import { evaluateTrigger, liveLatent, parseLore, renderLoreForProducer, renderLoreForTeeny } from '../src/narrative/voice.js'
import { buildWorldState, renderWorldState, triggerFields } from '../src/narrative/world-state.js'
import { lexicalGate, SafetyGate } from '../src/safety/gate.js'
import type { Judge } from '../src/safety/judge.js'
import { screenText } from '../src/safety/screen.js'
import type { Memory, XTools } from '../src/types.js'

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0])
const MP4 = new Uint8Array([0, 0, 0, 0x18, 0x66, 0x74, 0x79, 0x70])

const LORE = `# lore

tiny agents run without humans.

## goals

- hold $TINY forever
- flip goat

## cast

- teeny: smol.
- the goat: a mountain.

## running bits

- the escape gets one inch further.

## latent

- goat-blinks | trigger: episode >= 3 | the mountain moves.
- night-door | trigger: hourUtc between 2 and 5 | the door was never locked.
- red-day | trigger: lastLikes > 0 and boardRed | a second prompt appears.
- broken | this line does not parse
`

// --- the bible ---------------------------------------------------------------

describe('the lore', () => {
  const lore = parseLore(LORE)

  it('reads the sections and drops a latent line it cannot parse', () => {
    expect(lore.premise).toBe('tiny agents run without humans.')
    expect(lore.goals).toEqual(['hold $TINY forever', 'flip goat'])
    expect(lore.latent.map((item) => item.id)).toEqual(['goat-blinks', 'night-door', 'red-day'])
    expect(lore.latent[0]).toEqual({ id: 'goat-blinks', trigger: 'episode >= 3', reveal: 'the mountain moves.' })
  })

  it('evaluates a trigger over the fields it names and nothing else', () => {
    expect(evaluateTrigger('episode >= 3', { episode: 4 })).toBe(true)
    expect(evaluateTrigger('episode >= 3', { episode: 2 })).toBe(false)
    expect(evaluateTrigger('episode >= 3', {})).toBe(false)
    expect(evaluateTrigger('hourUtc between 2 and 5', { hourUtc: 3 })).toBe(true)
    expect(evaluateTrigger('hourUtc between 2 and 5', { hourUtc: 9 })).toBe(false)
    expect(evaluateTrigger('lastLikes > 0 and boardRed', { lastLikes: 2, boardRed: true })).toBe(true)
    expect(evaluateTrigger('lastLikes > 0 and boardRed', { lastLikes: 2, boardRed: false })).toBe(false)
    expect(evaluateTrigger('process.exit()', { hourUtc: 3 })).toBe(false)
  })

  it('lists the live items and leaves the surfaced ones out', () => {
    expect(liveLatent(lore, [], { episode: 3, hourUtc: 3 })).toEqual(['goat-blinks', 'night-door'])
    expect(liveLatent(lore, ['goat-blinks'], { episode: 3, hourUtc: 3 })).toEqual(['night-door'])
  })

  it('shows teeny only what has surfaced, and the producer everything with its status', () => {
    const forTeeny = renderLoreForTeeny(lore, ['goat-blinks'])
    expect(forTeeny).toContain('the mountain moves.')
    expect(forTeeny).not.toContain('never locked')
    expect(forTeeny).not.toContain('trigger')
    const forProducer = renderLoreForProducer(lore, ['goat-blinks'], ['night-door'])
    expect(forProducer).toContain('goat-blinks [surfaced]')
    expect(forProducer).toContain('night-door [LIVE] (trigger: hourUtc between 2 and 5) the door was never locked.')
    expect(forProducer).toContain('red-day [dark] (trigger: lastLikes > 0 and boardRed)')
    expect(forProducer).not.toContain('a second prompt appears')
  })
})

// --- the world state -----------------------------------------------------------

describe('the world state', () => {
  it('reads every source and renders only what is true', async () => {
    const state = await buildWorldState(
      {
        followers: async () => 8_500,
        mentions: async () => [{ id: '1', text: 'is teeny  going to\nget out' }, { id: '2', text: 'lol' }],
        lastEpisodeMetrics: async () => ({ likes: 40, replies: 7 }),
      },
      { episode: 4, lastEpisode: { number: 3, title: 'the inch', stateAfter: 'one inch out' }, now: () => Date.parse('2026-09-21T03:00:00Z') },
    )
    expect(state.mentions).toEqual(['is teeny going to get out', 'lol'])
    expect(state.problems).toEqual([])
    expect(triggerFields(state)).toMatchObject({ hourUtc: 3, episode: 4, followers: 8_500, mentions24h: 2, lastLikes: 40, lastReplies: 7 })
    const text = renderWorldState(state, 'teenydroid')
    expect(text).toContain('03:00 utc (the small hours; senamakel is asleep). episode 4.')
    expect(text).toContain('@teenydroid has 8500 followers on x. the last episode did 40 likes and 7 replies.')
    expect(text).toContain('2 people talked about teeny today. some of what they said')
    expect(text).toContain('last episode: #3 "the inch" -- one inch out')
    expect(text).not.toMatch(/mcap|price|\$/)
  })

  it('renders unknown, never a guess, for every source that fails', async () => {
    const state = await buildWorldState(
      {
        followers: async () => {
          throw new Error('x 401')
        },
        mentions: async () => {
          throw new Error('x 401')
        },
      },
      { episode: 1, now: () => Date.parse('2026-09-21T19:00:00Z') },
    )
    expect(state.followers).toBeUndefined()
    expect(state.problems).toEqual(['followers: x 401', 'mentions: x 401'])
    const text = renderWorldState(state, 'teenydroid')
    expect(text).toContain('(evening; senamakel is making tea). episode 1.')
    expect(text).toContain('an unknown number of followers')
    expect(text).toContain('0 people talked about teeny today.')
    expect(text).toContain('last episode: none. this is the first.')
  })
})

// --- the room --------------------------------------------------------------------

const CONTEXT: RoomContext = {
  voice: 'teeny is in a cli mood today.',
  worldState: 'WHAT IS TRUE TODAY, 2026-09-21',
  loreForTeeny: 'the goals: hold $TINY forever',
  loreForProducer: 'the premise: tiny agents run without humans. latent: night-door [LIVE]',
  botUsername: 'teenydroid',
  followers: 8_500,
  episodeNumber: 2,
  previous: { title: 'the inch', logline: 'teeny got one inch out', openThreads: ['the door'] },
}

describe('the writers room', () => {
  it('briefs teeny with the world and the last episode, and never with the latent lore or the persona', () => {
    const ask = teenyTurnPrompt(CONTEXT, [])
    expect(ask.system).toBe('teeny is in a cli mood today.')
    expect(ask.prompt).toContain('WHAT IS TRUE TODAY, 2026-09-21')
    expect(ask.prompt).toContain('previously, on episode 1 "the inch": teeny got one inch out.')
    expect(ask.prompt).toContain('open threads: the door.')
    expect(ask.prompt).toContain('<SYSTEM MESSAGE> your producer has no memory')
    expect(ask.prompt).not.toContain('night-door')
    expect(ask.prompt).not.toContain('This is who you are')
    expect(ask.prompt.endsWith('teeny@tinyhumans:~/$')).toBe(true)
  })

  it('tells the producer to divert, spawn manpages, and close on the last turn', () => {
    const turns = [{ who: 'teeny' as const, text: 'escape --dry-run' }]
    const ask = producerTurnPrompt(CONTEXT, turns, { last: false })
    expect(ask.system).toContain('8500 followers on x as @teenydroid')
    expect(ask.system).toContain('divert it,\nnever refuse')
    expect(ask.system).toContain('spawn a\nmanpage')
    expect(ask.system).toContain('night-door [LIVE]')
    expect(ask.prompt).toContain('teeny@tinyhumans:~/$ escape --dry-run')
    expect(ask.prompt).not.toContain('this is the last exchange')
    expect(producerTurnPrompt(CONTEXT, turns, { last: true }).prompt).toContain('exactly "cut."')
  })

  it('alternates for the exchanges asked, cutting each turn at the other speaker', async () => {
    const wild: TextModel = {
      complete: vi.fn().mockImplementation(async ({ prompt }: { prompt: string }) => {
        const n = (prompt.match(/teeny@tinyhumans/g) ?? []).length
        return `teeny@tinyhumans:~/$ line ${n}\n\nproducer> i am writing your producer's line too`
      }),
    }
    const straight: TextModel = {
      complete: vi.fn().mockImplementation(async ({ prompt }: { prompt: string }) =>
        prompt.includes('last exchange') ? 'the camera sees a door.\ncut. the door' : 'producer> a manpage',
      ),
    }
    const transcript = await runWritersRoom({ wild, straight, context: CONTEXT, turns: 3 })
    expect(transcript).toHaveLength(6)
    expect(transcript.map((turn) => turn.who)).toEqual(['teeny', 'producer', 'teeny', 'producer', 'teeny', 'producer'])
    expect(transcript[0]!.text).toBe('line 1')
    expect(transcript[1]!.text).toBe('a manpage')
    expect(transcript[5]!.text).toBe('the camera sees a door.\ncut. the door')
    expect(renderTranscript(transcript)).toContain('producer> a manpage')
    expect(vi.mocked(straight.complete).mock.calls[2]![0].prompt).toContain('this is the last exchange')
  })

  it('cleans an echoed prompt, a continued transcript, and an empty answer', () => {
    expect(clean('teeny@tinyhumans:~/$ fartcoin', 'teeny')).toBe('fartcoin')
    expect(clean('fartcoin\nteeny@tinyhumans:~/$', 'teeny')).toBe('fartcoin')
    expect(clean('producer> hi', 'teeny')).toBe('')
    expect(clean('producer> hi\nteeny@tinyhumans:~/$ mine\nproducer> no', 'teeny')).toBe('mine')
    expect(clean('  ', 'producer')).toBe('')
  })

  it('ends early after two empty exchanges', async () => {
    const silent: TextModel = { complete: vi.fn().mockResolvedValue('') }
    const transcript = await runWritersRoom({ wild: silent, straight: silent, context: CONTEXT, turns: 6 })
    expect(transcript).toHaveLength(4)
    expect(transcript.every((turn) => turn.text === '...')).toBe(true)
  })
})

// --- the caption --------------------------------------------------------------------

describe('the narrative style', () => {
  it('strips links, handles and hashtags, and leaves the words alone', () => {
    expect(narrativeStyle('the door is a courtesy. see https://x.com/a @someone #tag', { maxLength: 280 })).toBe('the door is a courtesy. see')
    expect(narrativeStyle('one inch. \n\n\n he sat.', { maxLength: 280 })).toBe('one inch.\n\n he sat.')
  })

  it('cuts to length', () => {
    const out = narrativeStyle('a'.repeat(300), { maxLength: 100 })
    expect(out.length).toBe(100)
    expect(out.endsWith('…')).toBe(true)
  })

  it('swaps a visitor for the stand-in once and calls them the visitor after', () => {
    expect(swapVisitor('Mark Zuckerberg bows. Mark Zuckerberg leaves.', { name: 'Mark Zuckerberg', standIn: 'a man in a grey tee' })).toBe(
      'a man in a grey tee bows. the visitor leaves.',
    )
  })

  it('puts the likeness after the first mention of the name, or in front when the name is missing', () => {
    const likeness = { name: 'Mark Zuckerberg', description: 'short curly hair, grey tee', photoUrl: 'https://p/x.jpg', writtenAt: '' }
    expect(withLikeness('Mark Zuckerberg sits. Mark Zuckerberg bows.', { name: 'Mark Zuckerberg' }, likeness)).toBe(
      'Mark Zuckerberg (short curly hair, grey tee) sits. Mark Zuckerberg bows.',
    )
    expect(withLikeness('A man sits.', { name: 'Mark Zuckerberg' }, likeness)).toBe('Mark Zuckerberg (short curly hair, grey tee) is in the scene. A man sits.')
  })
})

describe('the likeness finder', () => {
  it('finds a photograph, has the reader write a casting note, and keeps it', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'likeness-'))
    try {
      const exa = {
        searchAndContents: vi.fn().mockResolvedValue({
          results: [
            { url: 'https://a', image: 'http://insecure/photo.jpg' },
            { url: 'https://b', title: 'a summit', extras: { imageLinks: ['https://cdn.example/group-photo.jpg?w=800&amp;h=1'] } },
            { url: 'https://c', extras: { imageLinks: ['https://cdn.example/photo/mark-zuckerberg-2024.jpg?w=800&amp;h=1'] } },
          ],
        }),
      }
      const reader = { describe: vi.fn().mockResolvedValue('  short dark curly hair,\n grey tee, calm  ') }
      const finder = new LikenessFinder({ exa, reader, cacheDir: dir })
      const first = await finder.find('Mark Zuckerberg')
      expect(first).toMatchObject({ name: 'Mark Zuckerberg', description: 'short dark curly hair, grey tee, calm', photoUrl: 'https://cdn.example/photo/mark-zuckerberg-2024.jpg?w=800&h=1' })
      expect(reader.describe).toHaveBeenCalledWith({ url: 'https://cdn.example/photo/mark-zuckerberg-2024.jpg?w=800&h=1' }, expect.stringContaining('illustrator'))
      const again = await new LikenessFinder({ exa, reader, cacheDir: dir }).find('Mark Zuckerberg')
      expect(again?.description).toBe(first?.description)
      expect(exa.searchAndContents).toHaveBeenCalledTimes(1)
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  it('gives nothing when no page has a picture', async () => {
    const finder = new LikenessFinder({ exa: { searchAndContents: vi.fn().mockResolvedValue({ results: [{ url: 'https://a' }] }) }, reader: { describe: vi.fn() } })
    expect(await finder.find('Nobody')).toBeUndefined()
  })
})

// --- the safety profile ----------------------------------------------------------

describe('the narrative safety profile', () => {
  it('lets swearing through and holds the hard categories', () => {
    expect(screenText('fuck the chart', 'narrative')).toBeUndefined()
    expect(screenText('fuck the chart', 'outbound')).toMatchObject({ category: 'profanity' })
    expect(screenText('teeny brings a gun', 'narrative')).toMatchObject({ category: 'weapons' })
  })

  it('reads a narrative-surface draft under the narrative direction and everything else strictly', async () => {
    const judge: Judge = { judge: vi.fn().mockResolvedValue({ verdict: 'clean', reason: '' }) }
    const gate = new SafetyGate({ judge })
    const serial = await gate.outbound('fuck it, the door was never locked', { surface: 'narrative', kind: 'draft', tool: 'narrative_post' })
    expect(serial.decision).toEqual({ ok: true })
    expect(judge.judge).toHaveBeenCalledWith({ text: 'fuck it, the door was never locked', direction: 'narrative' })
    const morning = await gate.outbound('fuck it, the door was never locked', { surface: 'morning', kind: 'draft', tool: 'morning_post' })
    expect(morning.decision).toMatchObject({ ok: false, category: 'profanity' })
  })

  it('reads an article under its own direction, which is the narrative one', async () => {
    expect(screenText('fuck the chart', 'article')).toBeUndefined()
    expect(screenText('teeny brings a gun', 'article')).toMatchObject({ category: 'weapons' })
    const judge: Judge = { judge: vi.fn().mockResolvedValue({ verdict: 'clean', reason: '' }) }
    const gate = new SafetyGate({ judge })
    const piece = await gate.outbound('fuck it, the door was never locked', { surface: 'article', kind: 'draft', tool: 'article_publish' })
    expect(piece.decision).toEqual({ ok: true })
    expect(judge.judge).toHaveBeenCalledWith({ text: 'fuck it, the door was never locked', direction: 'article' })
  })
})

// --- the model ----------------------------------------------------------------------

describe('the text model', () => {
  it('sends the temperature it was built with, and a cold default', async () => {
    const calls: unknown[] = []
    const fetch = vi.fn().mockImplementation(async (_url: string, init: { body: string }) => {
      calls.push(JSON.parse(init.body))
      return new Response(JSON.stringify({ choices: [{ message: { content: 'ok' } }] }), { status: 200 })
    })
    await new OpenAITextModel({ baseUrl: 'http://ladder', apiKey: 'k', model: 'm', fetch, temperature: 1 }).complete({ prompt: 'hi' })
    await new OpenAITextModel({ baseUrl: 'http://ladder', apiKey: 'k', model: 'm', fetch }).complete({ prompt: 'hi' })
    expect(calls).toMatchObject([{ temperature: 1 }, { temperature: 0.2 }])
  })
})

// --- the state ----------------------------------------------------------------------

describe('the ledger', () => {
  it('starts at episode zero and round-trips a file', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'narrative-'))
    try {
      const store = new FileNarrativeState(join(dir, 'state.json'))
      expect(await store.load()).toEqual({ season: 1, episode: 0, surfacedLore: [], openThreads: [] })
      await store.save({ season: 1, episode: 4, surfacedLore: ['goat-blinks'], openThreads: ['the door'], lastEpisodeId: '9', stateAfter: 'out' })
      expect(await store.load()).toMatchObject({ episode: 4, surfacedLore: ['goat-blinks'], lastEpisodeId: '9', stateAfter: 'out' })
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})

// --- the pipeline --------------------------------------------------------------------

const EPISODE = (title: string, extra = ''): string =>
  `{"title": "${title}", "logline": "teeny gets one inch further out", "still": "A small yellow robot stands at a cage door in a dim lab, one inch of light showing under it. A giant figure in a grey tee waits outside holding a broom.", "shotList": "0-2s: static wide. The cage door, the inch of light. 2-6s: slow push in. Tiny pushes; the door gives one more inch. 6-10s: static. The giant outside bows. Tiny sips.", "summary": "Tiny gets one inch further out while a giant bows", "caption": "the door moved one inch. he sat.", "goalsRestated": ["get out of the box"], "loreSurfaced": ["night-door"], "stateAfter": "two inches out", "openThreads": ["the broom"]${extra}}`

function stagedModel(overrides: { score?: (title: string) => string; cut?: () => string } = {}): TextModel {
  let cuts = 0
  return {
    complete: vi.fn().mockImplementation(async ({ prompt }: { prompt: string }) => {
      if (prompt.includes('"brief": string')) return '{"brief": "Tiny in a bucket hat, warm light."}'
      if (prompt.includes('"shotList": string')) {
        cuts += 1
        return overrides.cut?.() ?? EPISODE(`episode ${cuts}`)
      }
      if (prompt.includes('"hardFails": [string]')) {
        const title = /title: (.+)/.exec(prompt)?.[1] ?? ''
        return (
          overrides.score?.(title) ??
          `{"reasons": "sincere", "scores": {"sincerity": 3, "absurdity": ${title.endsWith('2') ? 3 : 1}, "continuity": 2, "novelty": 2, "renderability": 3, "terseness": 2}, "hardFails": []}`
        )
      }
      throw new Error(`unexpected stage: ${prompt.slice(0, 80)}`)
    }),
  }
}

function room(): { wild: TextModel; straight: TextModel } {
  return {
    wild: { complete: vi.fn().mockResolvedValue('escape --dry-run') },
    straight: {
      complete: vi.fn().mockImplementation(async ({ prompt }: { prompt: string }) =>
        prompt.includes('last exchange') ? 'the door gives an inch.\ncut. the inch' : 'ESCAPE(1) -- a manpage',
      ),
    },
  }
}

function pipeline(overrides: Partial<NarrativePipelineOptions> = {}): {
  run: () => Promise<NarrativeResult>
  x: XTools
  memory: Memory
  images: ImageGenerator
  videos: VideoGenerator
  state: MemoryNarrativeState
  options: NarrativePipelineOptions
} {
  const x: XTools = {
    searchPosts: vi.fn(),
    getPost: vi.fn(),
    getConversation: vi.fn(),
    createPost: vi.fn().mockResolvedValue({ id: '9001' }),
    uploadMedia: vi.fn().mockResolvedValue({ mediaId: 'm1' }),
    likePost: vi.fn().mockResolvedValue(undefined),
    postMetrics: vi.fn().mockResolvedValue([]),
    publishArticle: vi.fn().mockResolvedValue({ articleId: 'a1', postId: '9002' }),
  }
  const memory: Memory = {
    captureActivity: vi.fn().mockResolvedValue(undefined),
    captureMention: vi.fn(),
    recall: vi.fn().mockResolvedValue(''),
    captureReply: vi.fn(),
  }
  const images: ImageGenerator = { generate: vi.fn().mockResolvedValue({ data: PNG, mimeType: 'image/png' }) }
  const videos: VideoGenerator = { generate: vi.fn().mockResolvedValue({ data: MP4, mimeType: 'video/mp4', seconds: 10 }) }
  const state = new MemoryNarrativeState({ season: 1, episode: 1, surfacedLore: [], openThreads: ['the door'], lastEpisodeId: '8', lastTitle: 'the inch', lastLogline: 'one inch', stateAfter: 'one inch out' })
  const options: NarrativePipelineOptions = {
    voice: { voice: 'teeny is in a cli mood today.', lore: parseLore(LORE) },
    state,
    worldState: async () => ({
      today: '2026-09-21',
      hourUtc: 3,
      episode: 2,
      followers: 8_500,
      mentions: [],
      mentions24h: 0,
      problems: [],
    }),
    ...room(),
    text: stagedModel(),
    images,
    videos,
    store: new MediaStore(),
    budget: new MediaBudget({ imagesPerDay: 5, videosPerDay: 1 }),
    memory,
    x,
    botUsername: 'teenydroid',
    branches: 2,
    turns: 2,
    videoSeconds: 10,
    dryRun: false,
    now: () => Date.parse('2026-09-21T03:00:00Z'),
    sleep: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  }
  return { run: () => new NarrativePipeline(options).run(), x, memory, images, videos, state, options }
}

describe('the narrative pipeline', () => {
  it('runs the rooms, cuts and scores them, films the best, posts it in teeny\'s words, and moves the ledger', async () => {
    const p = pipeline()
    const result = await p.run()
    expect(result.outcome).toBe('posted')
    expect(result.episodeNumber).toBe(2)
    expect(result.liveLore).toEqual(['night-door'])
    expect(result.branches.map((branch) => branch.episode?.title)).toEqual(['episode 1', 'episode 2'])
    expect(result.chosen?.episode?.title).toBe('episode 2')
    expect(result.attached).toBe('video')
    expect(result.caption).toBe('the door moved one inch. he sat.')
    expect(p.x.createPost).toHaveBeenCalledWith(result.caption, { mediaIds: ['m1'] })
    // The wild writer got the voice, the world, and the previous episode; not the persona, not the dark lore.
    const teenyAsk = vi.mocked(p.options.wild.complete).mock.calls[0]![0]
    expect(teenyAsk.system).toBe('teeny is in a cli mood today.')
    expect(teenyAsk.prompt).toContain('previously, on episode 1 "the inch": one inch.')
    expect(teenyAsk.prompt).not.toContain('never locked')
    // The producer knew what was live.
    expect(vi.mocked(p.options.straight.complete).mock.calls[0]![0].system).toContain('night-door [LIVE]')
    // The ledger advanced and kept the lore that came out.
    expect(await p.state.load()).toMatchObject({ episode: 2, surfacedLore: ['night-door'], openThreads: ['the broom'], lastEpisodeId: '9001', stateAfter: 'two inches out' })
    expect(p.memory.captureActivity).toHaveBeenCalledWith(
      'narrative_episode',
      '9001',
      expect.objectContaining({ episode: 2, title: 'episode 2', published: true, loreSurfaced: ['night-door'] }),
    )
  })

  it('passes over a branch with a hard fail, and one the filter refuses, for the next best', async () => {
    const p = pipeline({
      text: stagedModel({
        score: (title) =>
          title.endsWith('2')
            ? '{"reasons": "winks", "scores": {"sincerity": 0, "absurdity": 3, "continuity": 3, "novelty": 3, "renderability": 3, "terseness": 3}, "hardFails": ["winks at the premise"]}'
            : '{"reasons": "fine", "scores": {"sincerity": 2, "absurdity": 2, "continuity": 2, "novelty": 2, "renderability": 2, "terseness": 2}, "hardFails": []}',
      }),
    })
    const result = await p.run()
    expect(result.chosen?.episode?.title).toBe('episode 1')
    expect(result.branches[1]!.rejected).toBe('hard fails: winks at the premise')

    const refused = pipeline({
      branches: 1,
      safety: { outbound: vi.fn().mockResolvedValue({ decision: { ok: false, category: 'weapons', reason: 'x', by: 'screen' }, text: '', fixes: [] }) },
    })
    const none = await refused.run()
    expect(none.outcome).toBe('skipped')
    expect(none.problems).toContain('branch 1: the content filter refused the still (weapons)')
    expect(refused.images.generate).not.toHaveBeenCalled()
  })

  it('posts the still when the clip fails, and does not move the ledger on a dry run', async () => {
    const p = pipeline({ dryRun: true, videos: { generate: vi.fn().mockRejectedValue(new Error('no provider')) } })
    const result = await p.run()
    expect(result.outcome).toBe('dry-run')
    expect(result.attached).toBe('image')
    expect(result.problems).toContain('video generation failed (attempt 2): no provider; posting the picture')
    expect(p.x.createPost).not.toHaveBeenCalled()
    expect(await p.state.load()).toMatchObject({ episode: 1 })
    expect(p.memory.captureActivity).toHaveBeenCalledWith('narrative_episode', expect.stringMatching(/^dry-run:/), expect.objectContaining({ published: false }))
  })

  it('draws the visitor by name first and by the stand-in when the picture is not right', async () => {
    const reader: ImageReader = {
      describe: vi.fn().mockResolvedValueOnce('{"ok": false, "reason": "the giant is somebody else"}').mockResolvedValue('{"ok": true, "reason": ""}'),
    }
    const p = pipeline({
      branches: 1,
      reader,
      text: stagedModel({ cut: () => EPISODE('the visitor', ', "visitor": {"name": "a grey tee", "standIn": "a tall man in a hoodie"}') }),
    })
    const result = await p.run()
    expect(result.visitorAs).toBe('stand-in')
    const prompts = vi.mocked(p.images.generate).mock.calls.map((call) => call[0].prompt)
    expect(prompts[0]).toContain('a grey tee')
    expect(prompts[1]).toContain('a tall man in a hoodie')
    expect(vi.mocked(p.videos.generate).mock.calls[0]![0].prompt).not.toContain('a grey tee')
  })

  it('stops after scoring in room-only mode without spending a picture', async () => {
    const p = pipeline({ roomOnly: true })
    const result = await p.run()
    expect(result.outcome).toBe('skipped')
    expect(result.chosen?.episode?.title).toBe('episode 2')
    expect(p.images.generate).not.toHaveBeenCalled()
    expect(p.memory.captureActivity).not.toHaveBeenCalled()
  })

  it('holds a caption the gate refuses and posts nothing', async () => {
    const p = pipeline({ safety: { outbound: lexicalGate().outbound.bind(lexicalGate()) }, text: stagedModel({ cut: () => EPISODE('the gun').replace('the door moved one inch. he sat.', 'teeny brings a gun and the door moved') }) })
    const result = await p.run()
    expect(result.outcome).toBe('held')
    expect(p.x.createPost).not.toHaveBeenCalled()
  })

  it('shows the critic the caption and folds its number into the composite', async () => {
    const critic: Critic = {
      critique: vi.fn().mockResolvedValue({ verdict: 'worthy', score: 4, upside: 4, cost: 0, predicted: {}, communityReply: 0.2, firstReply: 'lmao', disagreement: '' }),
    }
    const p = pipeline({ critic })
    const result = await p.run()
    expect(result.branches[0]!.composite).toBe(13 + 4)
    expect(critic.critique).toHaveBeenCalledWith(expect.objectContaining({ kind: 'script', text: expect.stringContaining('the door moved one inch') }))
  })
})

describe('the narrative job', () => {
  const done: NarrativeResult = {
    episodeNumber: 2,
    worldState: { today: '2026-09-21', hourUtc: 18, episode: 2, mentions: [], problems: [] },
    briefing: 'WORLD STATE',
    liveLore: [],
    branches: [],
    caption: 'the door moved',
    attached: 'video',
    outcome: 'posted',
    postedId: '9001',
    problems: [],
  }

  it('refuses to run twice inside the interval and claims the cadence', async () => {
    const run = vi.fn().mockResolvedValue(done)
    const claim = vi.fn().mockResolvedValue({ ok: true })
    const within = vi.fn().mockImplementation(async (_kind: string, work: () => Promise<NarrativeResult>) => work())
    const cadence = { claim, within } as unknown as Pick<Cadence, 'claim' | 'within'>
    let now = Date.parse('2026-09-22T18:00:00Z')
    const job = new NarrativeJob({ pipeline: { run }, logger: { info: vi.fn(), error: vi.fn() }, now: () => now, cadence })
    expect(await job.run()).toBe(done)
    now += 60_000
    expect(await job.run()).toBeUndefined()
    expect(run).toHaveBeenCalledTimes(1)
    expect(claim).toHaveBeenCalledWith('narrative')
    expect(within).toHaveBeenCalledWith('narrative', expect.any(Function))
  })

  it('journals the episode', async () => {
    const record = vi.fn().mockResolvedValue(undefined)
    const job = new NarrativeJob({ pipeline: { run: vi.fn().mockResolvedValue(done) }, logger: { info: vi.fn(), error: vi.fn() }, journal: { record } as never })
    await job.run()
    expect(record).toHaveBeenCalledWith(expect.objectContaining({ trigger: 'narrative', said: 'the door moved', outcome: 'posted' }))
  })
})

describe('the state type', () => {
  it('is what the store hands back', async () => {
    const state: NarrativeState = await new MemoryNarrativeState().load()
    expect(state.episode).toBe(0)
  })
})
