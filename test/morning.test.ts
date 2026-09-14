import { describe, expect, it, vi } from 'vitest'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { loadConfig } from '../src/config.js'
import type { Cadence } from '../src/cadence/cadence.js'
import { MorningJob } from '../src/jobs/morning-job.js'
import type { Journal } from '../src/journal/journal.js'
import { ReferenceSheet, withSeed, type TextModel } from '../src/media/reference.js'
import { MediaBudget, MediaStore } from '../src/media/store.js'
import type { ImageGenerator, ImageReader, VideoGenerator } from '../src/media/types.js'
import { lexicalGate, SafetyGate } from '../src/safety/gate.js'
import type { HoldLog } from '../src/safety/holds.js'
import { LadderVideoGenerator, OpenAIVideoGenerator } from '../src/media/videos.js'
import { ExaNewsSource, gatherNews, isPolitical, renderStories, type NewsSource, type Story } from '../src/morning/news.js'
import { MorningPipeline, type MorningPipelineOptions, type MorningResult } from '../src/morning/pipeline.js'
import type { Critic, Critique } from '../src/critic/algo.js'
import { askJson, extractJson, pickSchema, scriptPrompt } from '../src/morning/stages.js'
import type { Memory, XMedia, XTools } from '../src/types.js'

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0])
const MP4 = new Uint8Array([0, 0, 0, 0x18, 0x66, 0x74, 0x79, 0x70])

function story(overrides: Partial<Story> & { title: string }): Story {
  return {
    url: `https://news.example/${overrides.title.replace(/\W+/g, '-').toLowerCase()}`,
    source: 'news.example',
    excerpt: '',
    topic: 'ai',
    ...overrides,
  }
}

describe('the political filter', () => {
  it('drops a story about governments, wars and named politicians, wherever the word sits', () => {
    expect(isPolitical(story({ title: 'Senate passes AI bill' }))).toBe(true)
    expect(isPolitical(story({ title: 'New model released', excerpt: 'The White House responded.' }))).toBe(true)
    expect(isPolitical(story({ title: 'Trump on crypto' }))).toBe(true)
    expect(isPolitical(story({ title: 'Chip tariffs land' }))).toBe(true)
    expect(isPolitical(story({ title: 'War in the east' }))).toBe(true)
    // Slipped through the first rehearsal.
    expect(isPolitical(story({ title: 'Washington scrambles to meet calls for AI guardrails', excerpt: 'Lawmakers are seeking to address...' }))).toBe(true)
    expect(isPolitical(story({ title: 'Feds Have Now Frozen $938M in Scam Crypto' }))).toBe(true)
  })

  it('keeps ordinary AI, crypto and tech stories, and matches whole words only', () => {
    expect(isPolitical(story({ title: 'Open-source agent framework hits 40k stars' }))).toBe(false)
    expect(isPolitical(story({ title: 'Solana devs ship a new runtime' }))).toBe(false)
    // "warehouse" is not "war"; "presidents" of companies are also not wanted, but "presidential" is caught above.
    expect(isPolitical(story({ title: 'Robots sort a warehouse' }))).toBe(false)
    expect(isPolitical(story({ title: 'A trumpet-playing robot' }))).toBe(false)
  })
})

describe('gathering the news', () => {
  it('searches every topic, drops politics, folds duplicates, and survives a failed search', async () => {
    const source: NewsSource = {
      read: vi.fn().mockResolvedValue(''),
      search: vi
        .fn()
        .mockResolvedValueOnce([
          story({ title: 'A new open model', topic: 'ai' }),
          story({ title: 'Congress hearing on AI', topic: 'ai' }),
        ])
        .mockRejectedValueOnce(new Error('exa down'))
        .mockResolvedValueOnce([
          story({ title: 'A new open model', topic: 'tech', url: 'https://news.example/a-new-open-model?utm=x' }),
          story({ title: 'Something about phones', topic: 'tech' }),
        ]),
    }
    const { stories, dropped } = await gatherNews(source, { topics: ['ai', 'crypto', 'tech'], days: 2, perTopic: 5 })
    expect(stories.map((entry) => entry.title)).toEqual(['A new open model', 'Something about phones'])
    expect(dropped).toBe(1)
    expect(source.search).toHaveBeenCalledTimes(3)
    expect(source.search).toHaveBeenCalledWith('crypto', { days: 2, limit: 5 })
  })

  it('numbers the stories from one, which is the handle the model picks by', () => {
    const rendered = renderStories([
      story({ title: 'First', publishedAt: '2026-09-14T06:00:00Z', excerpt: 'It happened.' }),
      story({ title: 'Second', topic: 'crypto' }),
    ])
    expect(rendered).toContain('[1] First\n    news.example, 2026-09-14 (ai)\n    It happened.')
    expect(rendered).toContain('[2] Second\n    news.example (crypto)')
  })

  it('asks Exa for recent news with highlights, and reads what it gets back', async () => {
    const searchAndContents = vi.fn().mockResolvedValue({
      results: [
        { title: ' A story ', url: 'https://www.example.com/a', publishedDate: '2026-09-14', highlights: ['one', 'two'] },
        { title: null, url: 'https://example.com/untitled' },
      ],
    })
    const getContents = vi.fn().mockResolvedValue({ results: [{ url: 'https://www.example.com/a', text: 'The  full\n article.' }] })
    const source = new ExaNewsSource({ searchAndContents, getContents } as never)
    await expect(source.read('https://www.example.com/a', { maxCharacters: 500 })).resolves.toBe('The full article.')
    expect(getContents).toHaveBeenCalledWith(['https://www.example.com/a'], { text: { maxCharacters: 500 } })
    const found = await source.search('ai news', { days: 2, limit: 4 })
    expect(found).toEqual([
      {
        title: 'A story',
        url: 'https://www.example.com/a',
        source: 'example.com',
        publishedAt: '2026-09-14',
        excerpt: 'one … two',
        topic: 'ai news',
      },
    ])
    expect(searchAndContents).toHaveBeenCalledWith(
      'ai news',
      expect.objectContaining({ numResults: 4, category: 'news', startPublishedDate: expect.any(String) }),
    )
  })
})

describe('asking for JSON', () => {
  it('finds the first balanced object, in prose or a code fence, and nothing else', () => {
    expect(extractJson('Sure! ```json\n{"a": 1, "b": {"c": "}"}}\n```')).toEqual({ a: 1, b: { c: '}' } })
    expect(extractJson('no json here')).toBeUndefined()
    expect(extractJson('{"broken": ')).toBeUndefined()
  })

  it('sends a rejected shape back once with the complaint, then gives up', async () => {
    const model: TextModel = {
      complete: vi
        .fn()
        .mockResolvedValueOnce('{"story": "three"}')
        .mockResolvedValueOnce('{"story": 3, "headline": "h", "topic": "ai", "angle": "a", "visual": "v"}'),
    }
    await expect(askJson(model, { system: 's', prompt: 'p', schema: pickSchema })).resolves.toMatchObject({ story: 3 })
    const second = (model.complete as ReturnType<typeof vi.fn>).mock.calls[1]?.[0] as { prompt: string }
    expect(second.prompt).toContain('previous answer was rejected (story:')

    // A call that fails outright is tried once more with the same prompt.
    const flaky: TextModel = {
      complete: vi
        .fn()
        .mockRejectedValueOnce(new DOMException('The operation was aborted due to timeout', 'TimeoutError'))
        .mockResolvedValueOnce('{"story": 1, "headline": "h", "topic": "ai", "angle": "a", "visual": "v"}'),
    }
    await expect(askJson(flaky, { system: 's', prompt: 'p', schema: pickSchema })).resolves.toMatchObject({ story: 1 })
    const dead: TextModel = { complete: vi.fn().mockRejectedValue(new Error('502')) }
    await expect(askJson(dead, { system: 's', prompt: 'p', schema: pickSchema })).rejects.toThrow('502')
    expect(dead.complete).toHaveBeenCalledTimes(2)

    const hopeless: TextModel = { complete: vi.fn().mockResolvedValue('nope') }
    await expect(askJson(hopeless, { system: 's', prompt: 'p', schema: pickSchema })).rejects.toThrow(/no JSON object/)
    expect(hopeless.complete).toHaveBeenCalledTimes(2)
  })
})

describe('the script stage', () => {
  it('writes the script as Tiny, with the persona and its skills in front of it', () => {
    const { system, prompt } = scriptPrompt(
      { persona: 'SOUL\n<skill name="video-script">beats</skill>', botUsername: 'TinyMascot', today: '2026-09-14' },
      {
        pick: { story: 1, headline: 'A model launched', topic: 'ai', angle: 'local-first has no status page', visual: 'a board' },
        scene: { concepts: ['a dead status board'], scene: 'A status board over a stage.', storyCue: 'the board', alt: 'a board' },
        seconds: 10,
      },
    )
    expect(system).toMatch(/^SOUL\n<skill name="video-script">beats<\/skill>\n/)
    expect(system).toContain('filming')
    expect(prompt).toContain('10-second script')
    expect(prompt).toContain('local-first has no status page')
  })
})

describe('the seed picture', () => {
  const dir = mkdtempSync(join(tmpdir(), 'teeny-morning-'))
  const a = join(dir, 'a.png')
  const b = join(dir, 'b.png')
  writeFileSync(a, PNG)
  writeFileSync(b, PNG)

  it('is one of the reference pictures with its own description, chosen by the random given', async () => {
    const reader: ImageReader = {
      describe: vi.fn().mockImplementation(async (input: { data: Uint8Array }) => `look ${input.data.byteLength}`),
    }
    const text: TextModel = { complete: vi.fn().mockResolvedValue('Tiny is a robot.') }
    const cachePath = join(dir, 'sheet.json')
    const sheet = new ReferenceSheet({ urls: [a, b], reader, text, cachePath })
    const first = await sheet.seed(() => 0)
    const last = await sheet.seed(() => 0.999)
    expect(first).toEqual({ url: a, look: expect.stringMatching(/^look/) })
    expect(last?.url).toBe(b)

    // The looks come back from the cache too, so a restart still has seeds.
    const again = new ReferenceSheet({ urls: [a, b], reader: { describe: vi.fn() }, text: { complete: vi.fn() }, cachePath })
    await expect(again.seed(() => 0)).resolves.toEqual(first)

    const none = new ReferenceSheet({ urls: [], reader, text })
    await expect(none.seed()).resolves.toBeUndefined()
  })

  it('goes between the sheet and the scene in the prompt', () => {
    expect(withSeed('Tiny is a robot.', { url: 'x', look: 'a beach' }, 'Tiny codes')).toBe(
      'Tiny is a robot.\n\nStyle reference, one of Tiny\'s official pictures (match its rendering, palette and mood, not its scene): a beach\n\nScene: Tiny codes',
    )
    expect(withSeed('Tiny is a robot.', undefined, 'Tiny codes')).toBe('Tiny is a robot.\n\nScene: Tiny codes')
    expect(withSeed(undefined, { url: 'x', look: 'a beach' }, 'Tiny codes')).not.toContain('Tiny is')
  })

  it('is the part dropped when the prompt would be too long for the generator, and the scene is cut last', () => {
    const sheet = 'Tiny is a robot. '.repeat(20).trim()
    const seed = { url: 'x', look: 'y'.repeat(500) }
    const scene = 'Tiny codes at dawn. '.repeat(30).trim()
    const fitted = withSeed(sheet, seed, scene, { maxLength: 1_000 })
    expect(fitted.length).toBeLessThanOrEqual(1_000)
    expect(fitted).not.toContain('Style reference')
    expect(fitted).toMatch(/^Tiny is a robot\..*\n\nScene: Tiny codes at dawn\..*\.$/s)
    expect(withSeed(sheet, seed, 'short', { maxLength: 2_000 })).toContain('Style reference')
  })

  it('cleans up', () => {
    rmSync(dir, { recursive: true, force: true })
  })
})

describe('the seconds hint', () => {
  function json(body: unknown, status = 200): Response {
    return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
  }

  it('reaches the ladder as OpenAI spells it, and only when asked for', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(json({ id: 'j', status: 'completed', output: { video_url: 'https://cdn.example/j.mp4' } }, 202))
      .mockResolvedValueOnce(new Response(MP4, { status: 200 }))
      .mockResolvedValueOnce(json({ id: 'k', status: 'completed', output: { video_url: 'https://cdn.example/k.mp4' } }, 202))
      .mockResolvedValueOnce(new Response(MP4, { status: 200 }))
    const videos = new LadderVideoGenerator({ baseUrl: 'http://l/v1', apiKey: 'k', model: 'video', fetch: fetchMock as never })
    await videos.generate({ prompt: 'Tiny waves hello', seconds: 12 })
    await videos.generate({ prompt: 'Tiny waves hello' })
    const bodies = (fetchMock.mock.calls as Array<[string, RequestInit]>)
      .filter(([, init]) => init?.body)
      .map(([, init]) => JSON.parse(init.body as string) as Record<string, unknown>)
    expect(bodies[0]).toMatchObject({ seconds: '12', duration: '12', video_length: 12, length: 12, duration_seconds: 12 })
    expect(bodies[1]).not.toHaveProperty('seconds')
    expect(bodies[1]).not.toHaveProperty('duration')
  })

  it('rounds up to a length Sora accepts', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(json({ id: 'v', status: 'completed' }))
      .mockResolvedValueOnce(new Response(MP4, { status: 200 }))
    const videos = new OpenAIVideoGenerator({ baseUrl: 'http://o/v1', apiKey: 'k', model: 'sora-2', fetch: fetchMock as never })
    const clip = await videos.generate({ prompt: 'Tiny waves hello', seconds: 10 })
    expect(JSON.parse((fetchMock.mock.calls[0] as [string, RequestInit])[1].body as string)).toMatchObject({ seconds: '12' })
    expect(clip.seconds).toBe(12)
  })
})

// --- the pipeline ------------------------------------------------------------

const STORIES = [
  story({ title: 'An open-source agent runtime crossed 40k stars', topic: 'ai', excerpt: 'Builders are flocking.' }),
  story({ title: 'A wallet shipped account abstraction', topic: 'crypto' }),
]

/** A model that answers each stage in turn by recognising the shape it was asked for. */
function stagedModel(overrides: Partial<Record<'pick' | 'scene' | 'script' | 'caption', string>> = {}): TextModel {
  return {
    complete: vi.fn().mockImplementation(async ({ prompt }: { prompt: string }) => {
      if (prompt.includes('"brief": string')) {
        return '{"brief": "Tiny in a bucket hat and sunglasses, glowing nodes across the body, polished 3D, warm gold light."}'
      }
      if (prompt.includes('"story": number')) {
        return overrides.pick ?? '{"story": 1, "headline": "An agent runtime hit 40k stars", "topic": "ai", "angle": "open source wins", "visual": "a star counter ticking past 40k"}'
      }
      if (prompt.includes('"scene": string')) {
        return overrides.scene ?? '{"scene": "Tiny sits at a sunrise desk, a mug in hand, watching a star counter tick up on a tiny screen.", "storyCue": "the counter reading 40k stars", "alt": "Tiny at a desk at dawn."}'
      }
      if (prompt.includes('"script": string')) {
        return (
          overrides.script ??
          '{"script": "0-4s: static shot, Tiny sips from the mug as the sun rises. 4-10s: slow push in as the counter ticks. 10-12s: Tiny raises the mug to camera.", "summary": "Tiny sips coffee at sunrise, then raises the mug to say gm"}'
        )
      }
      if (prompt.includes('"text": string')) {
        return overrides.caption ?? '{"text": "gm builders.\\n\\nan open-source agent runtime crossed 40k stars overnight, reportedly. small robots, big repos.\\n\\nin today\'s clip Tiny raises a mug to all of you."}'
      }
      throw new Error(`unexpected stage: ${prompt.slice(0, 80)}`)
    }),
  }
}

function pipeline(overrides: Partial<Omit<MorningPipelineOptions, 'videos'>> & { videos?: VideoGenerator | undefined } = {}): {
  run: () => Promise<MorningResult>
  x: XTools
  memory: Memory
  images: ImageGenerator
  videos: VideoGenerator
  text: TextModel
} {
  const x: XTools = {
    searchPosts: vi.fn(),
    getPost: vi.fn(),
    getConversation: vi.fn(),
    createPost: vi.fn().mockResolvedValue({ id: '9001' }),
    uploadMedia: vi.fn().mockResolvedValue({ mediaId: 'm1' }),
    likePost: vi.fn().mockResolvedValue(undefined),
    postMetrics: vi.fn().mockResolvedValue([]),
  }
  const memory: Memory = {
    captureActivity: vi.fn().mockResolvedValue(undefined),
    captureMention: vi.fn(),
    recall: vi.fn().mockResolvedValue('yesterday: a post about robots'),
    captureReply: vi.fn(),
  }
  const images: ImageGenerator = { generate: vi.fn().mockResolvedValue({ data: PNG, mimeType: 'image/png' }) }
  const videos: VideoGenerator = { generate: vi.fn().mockResolvedValue({ data: MP4, mimeType: 'video/mp4', seconds: 12 }) }
  const text = overrides.text ?? stagedModel()
  const { videos: videoOverride, ...rest } = overrides
  const options: MorningPipelineOptions = {
    news: { search: vi.fn().mockResolvedValue(STORIES), read: vi.fn().mockResolvedValue('The runtime crossed 40,000 stars on Saturday.') },
    text,
    images,
    ...('videos' in overrides ? (videoOverride ? { videos: videoOverride } : {}) : { videos }),
    store: new MediaStore(),
    budget: new MediaBudget({ imagesPerDay: 5, videosPerDay: 1 }),
    memory,
    x,
    persona: 'You are Tiny.',
    botUsername: 'teenythebot',
    dryRun: false,
    topics: ['ai'],
    newsDays: 2,
    storiesPerTopic: 5,
    videoSeconds: 12,
    maxEmoji: 1,
    now: () => Date.parse('2026-09-14T08:00:00Z'),
    sleep: vi.fn().mockResolvedValue(undefined),
    ...rest,
  }
  return { run: () => new MorningPipeline(options).run(), x, memory, images, videos, text }
}

describe('the morning pipeline', () => {
  it('reads, picks, draws from a seed, scripts, films, captions, posts the clip, and remembers', async () => {
    const reference = {
      get: vi.fn().mockResolvedValue('Tiny is a small yellow robot.'),
      seed: vi.fn().mockResolvedValue({ url: 'assets/tiny/tiny-love.jpg', look: 'Tiny hugging a heart, warm palette' }),
    } as unknown as ReferenceSheet
    const { run, x, memory, images, videos } = pipeline({ reference })
    const result = await run()

    expect(result.outcome).toBe('posted')
    expect(result.postedId).toBe('9001')
    expect(result.attached).toBe('video')
    expect(result.story?.title).toContain('40k stars')
    expect(result.problems).toEqual([])

    // The image prompt is sheet, then the seed, then the scene; the video the same with the script.
    const imagePrompt = (images.generate as ReturnType<typeof vi.fn>).mock.calls[0]?.[0] as { prompt: string; aspect: string }
    // The seed reaches the generator briefed, not as the thousand-character illustrator's note.
    expect(imagePrompt.prompt).toMatch(/^Tiny is a small yellow robot\.\n\nStyle reference.*bucket hat.*\n\nScene: Tiny sits/s)
    expect(imagePrompt.prompt).not.toContain('hugging a heart')
    expect(imagePrompt.aspect).toBe('square')
    const videoPrompt = (videos.generate as ReturnType<typeof vi.fn>).mock.calls[0]?.[0] as { prompt: string; seconds: number }
    expect(videoPrompt.prompt).toContain('Scene: 0-4s: static shot')
    expect(videoPrompt.seconds).toBe(12)

    // The clip is what goes up, with the caption that describes it.
    expect(x.uploadMedia).toHaveBeenCalledWith({ data: MP4, mimeType: 'video/mp4' })
    expect(x.createPost).toHaveBeenCalledWith(expect.stringContaining('gm builders'), { mediaIds: ['m1'] })
    expect(result.caption).toContain("in today's clip")

    // Memory on both sides: recalled before, written back after.
    expect(memory.recall).toHaveBeenCalledTimes(2)
    expect(memory.captureActivity).toHaveBeenCalledWith(
      'morning_post',
      '9001',
      expect.objectContaining({
        text: result.caption,
        headline: 'An agent runtime hit 40k stars',
        topic: 'ai',
        seed: 'assets/tiny/tiny-love.jpg',
        clip: expect.stringContaining('raises the mug'),
        attached: 'video',
        published: true,
      }),
    )
  })

  it('tells the picker what it already posted, and the caption who it is talking to', async () => {
    const { run, text } = pipeline()
    await run()
    const calls = (text.complete as ReturnType<typeof vi.fn>).mock.calls as Array<[{ system: string; prompt: string }]>
    const scene = calls.find(([call]) => call.prompt.includes('"storyCue": string'))![0]
    expect(scene.prompt).toContain('The runtime crossed 40,000 stars on Saturday.')
    expect(scene.prompt).toContain('a star counter ticking past 40k')
    const pick = calls.find(([call]) => call.prompt.includes('"story": number'))![0]
    expect(pick.prompt).toContain('[1] An open-source agent runtime crossed 40k stars')
    expect(pick.prompt).toContain('yesterday: a post about robots')
    expect(pick.system).toContain('You are Tiny.')
    const caption = calls.find(([call]) => call.prompt.includes('"text": string'))![0]
    expect(caption.prompt).toContain('yesterday: a post about robots')
    // The clip is not narrated: the caption is told one is attached, and to leave it be.
    expect(caption.prompt).not.toContain('Tiny sips coffee at sunrise')
    expect(caption.prompt).toContain('Do NOT describe it')
    expect(caption.prompt).toContain('$TINY cashtag')
  })

  it('looks at the picture, redraws once when it is not Tiny, and keeps the second either way', async () => {
    const reader: ImageReader = {
      describe: vi
        .fn()
        .mockResolvedValueOnce('{"ok": false, "reason": "the robot is purple"}')
        .mockResolvedValueOnce('{"ok": true, "reason": "yellow, round, smiling"}'),
    }
    const budget = new MediaBudget({ imagesPerDay: 5, videosPerDay: 1 })
    const { run, images } = pipeline({ reader, budget })
    const result = await run()
    expect(images.generate).toHaveBeenCalledTimes(2)
    expect(reader.describe).toHaveBeenCalledWith({ data: PNG, mimeType: 'image/png' }, expect.stringContaining('Tiny'))
    expect(result.problems).toEqual(['redrew the picture: the robot is purple'])
    expect(budget.remaining('image')).toBe(3)

    // A verdict whose reason runs long, or whose JSON was cut off, still counts.
    const verbose: ImageReader = {
      describe: vi.fn().mockResolvedValue(`{"ok": true, "reason": "${'a very long explanation. '.repeat(30)}"}`),
    }
    const { run: wordy, images: drawnOnce } = pipeline({ reader: verbose })
    expect((await wordy()).problems).toEqual([])
    expect(drawnOnce.generate).toHaveBeenCalledTimes(1)
    const cutOff: ImageReader = { describe: vi.fn().mockResolvedValue('{"ok": false, "reason": "the robot is purp') }
    const { run: truncated, images: drawnTwice } = pipeline({ reader: cutOff })
    await truncated()
    expect(drawnTwice.generate).toHaveBeenCalledTimes(2)

    const stubborn: ImageReader = { describe: vi.fn().mockResolvedValue('No, it is a cat.') }
    const { run: again, images: drawn } = pipeline({ reader: stubborn })
    const kept = await again()
    expect(drawn.generate).toHaveBeenCalledTimes(2)
    expect(kept.attached).toBe('video')
    expect(kept.problems).toEqual(['redrew the picture: No, it is a cat.', 'kept a picture the check did not like: No, it is a cat.'])

    // Eyes that fail do not stop the morning.
    const blind: ImageReader = { describe: vi.fn().mockRejectedValue(new Error('vision down')) }
    const { run: trusting, images: once } = pipeline({ reader: blind })
    expect((await trusting()).problems).toEqual([])
    expect(once.generate).toHaveBeenCalledTimes(1)
  })

  it('retries a failed clip once, then posts the picture, and the caption knows it', async () => {
    const { run, x, videos } = pipeline()
    ;(videos.generate as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('rung timed out'))
    const result = await run()
    expect(result.attached).toBe('image')
    expect(result.outcome).toBe('posted')
    expect(result.problems).toEqual([
      'video generation failed (attempt 1): rung timed out',
      'video generation failed (attempt 2): rung timed out; posting the picture',
    ])
    expect(videos.generate).toHaveBeenCalledTimes(2)
    // A verbatim retry gets the same failed job back from the marketplace, so the second differs.
    const prompts = (videos.generate as ReturnType<typeof vi.fn>).mock.calls.map((call) => (call[0] as { prompt: string }).prompt)
    expect(prompts[1]).toBe(`${prompts[0]}\n\n(take 2)`)
    expect(x.uploadMedia).toHaveBeenCalledWith({ data: PNG, mimeType: 'image/png' })
  })

  it('posts the picture when video is off or the day\'s video is spent', async () => {
    const { run: withoutVideo } = pipeline({ videos: undefined })
    expect((await withoutVideo()).problems).toEqual(['video generation is off; posting the picture'])

    const budget = new MediaBudget({ imagesPerDay: 5, videosPerDay: 1 })
    budget.take('video')
    const { run: spent, videos } = pipeline({ budget })
    const result = await spent()
    expect(result.attached).toBe('image')
    expect(videos.generate).not.toHaveBeenCalled()
  })

  it('posts nothing when there is no picture, because the picture is the post', async () => {
    const { run, x, images, memory } = pipeline()
    ;(images.generate as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('no rung'))
    const result = await run()
    expect(result.outcome).toBe('skipped')
    expect(result.attached).toBe('none')
    expect(result.problems).toEqual([
      'image generation failed (attempt 1): no rung',
      'image generation failed (attempt 2): no rung',
      'nothing to attach, so nothing was posted',
    ])
    expect(x.createPost).not.toHaveBeenCalled()
    expect(memory.captureActivity).not.toHaveBeenCalled()
  })

  it('in a dry run does everything but upload and post, and remembers the rehearsal as one', async () => {
    const { run, x, memory, images, videos } = pipeline({ dryRun: true })
    const result = await run()
    expect(result.outcome).toBe('dry-run')
    expect(images.generate).toHaveBeenCalled()
    expect(videos.generate).toHaveBeenCalled()
    expect(x.uploadMedia).not.toHaveBeenCalled()
    expect(x.createPost).not.toHaveBeenCalled()
    expect(memory.captureActivity).toHaveBeenCalledWith(
      'morning_post',
      expect.stringMatching(/^dry-run:/),
      expect.objectContaining({ published: false }),
    )
  })

  it('carries on with the headline when the model names a story that is not on the list, or none at all', async () => {
    const { run } = pipeline({
      text: stagedModel({ pick: '{"story": 7, "headline": "made up", "topic": "tech", "angle": "a", "visual": "v"}' }),
    })
    const result = await run()
    expect(result.story).toBeUndefined()
    expect(result.problems[0]).toMatch(/chose story 7 of 2/)

    const { run: quiet } = pipeline({
      text: stagedModel({ pick: '{"story": 0, "headline": "OpenHuman", "topic": "ai", "angle": "ours", "visual": "v", "fallback": "the runtime"}' }),
    })
    const fallback = await quiet()
    expect(fallback.story).toBeUndefined()
    expect(fallback.pick.angle).toBe('ours (the runtime)')
    expect(fallback.outcome).toBe('posted')
  })

  it('sends a caption over the limit back once, then applies the house style', async () => {
    const long = 'x'.repeat(300)
    const model = stagedModel()
    const original = model.complete as ReturnType<typeof vi.fn>
    let captionCalls = 0
    const text: TextModel = {
      complete: vi.fn().mockImplementation(async (input: { system: string; prompt: string }) => {
        if (input.prompt.includes('"text": string')) {
          captionCalls += 1
          if (captionCalls === 1) return JSON.stringify({ text: long })
          expect(input.prompt).toContain('Your draft was 300 characters')
          return '{"text": "gm — short one 🌞🌞"}'
        }
        return original(input)
      }),
    }
    const { run } = pipeline({ text })
    const result = await run()
    expect(captionCalls).toBe(2)
    expect(result.caption.length).toBeLessThanOrEqual(280)
    expect(result.caption).not.toContain('—')
    // Two emoji in, one allowed: the house style trims rather than refusing.
    expect([...result.caption.matchAll(/🌞/g)]).toHaveLength(1)
  })

  it('reads the caption through the content filter before posting, and withholds the post when it is refused', async () => {
    const record = vi.fn().mockResolvedValue(undefined)
    const holds = { record, allowedLinks: vi.fn().mockResolvedValue([]) } as unknown as HoldLog
    const { run, x, memory, images, videos } = pipeline({
      safety: lexicalGate({ holds }),
      text: stagedModel({ caption: '{"text": "gm. best gun for home defence? tiny knows."}' }),
    })
    const result = await run()

    expect(result.outcome).toBe('held')
    expect(result.attached).toBe('video')
    expect(result.caption).toBe('')
    expect(result.problems).toEqual(['content filter withheld the caption: weapons'])
    // Generated, then stopped at the door: the picture and the clip are not posted bare.
    expect(images.generate).toHaveBeenCalled()
    expect(videos.generate).toHaveBeenCalled()
    expect(x.uploadMedia).not.toHaveBeenCalled()
    expect(x.createPost).not.toHaveBeenCalled()
    expect(memory.captureActivity).not.toHaveBeenCalled()
    // The caption itself is in the hold log, on the morning surface, for review.
    expect(record).toHaveBeenCalledWith(
      expect.objectContaining({
        direction: 'outbound',
        kind: 'draft',
        context: { surface: 'morning', kind: 'draft', tool: 'morning_post' },
        category: 'weapons',
        text: expect.stringContaining('best gun'),
      }),
    )
  })

  it('posts what the filter handed back, with a link off the allowlist cut, and notes the edit', async () => {
    const safety = lexicalGate()
    const outbound = vi.spyOn(safety, 'outbound')
    const { run, x } = pipeline({
      safety,
      text: stagedModel({ caption: '{"text": "gm builders. see https://totally-legit-claim.xyz/airdrop for the story."}' }),
    })
    const result = await run()
    expect(result.outcome).toBe('posted')
    expect(result.caption).toBe('gm builders. see for the story.')
    expect(x.createPost).toHaveBeenCalledWith('gm builders. see for the story.', { mediaIds: ['m1'] })
    expect(result.problems).toEqual([expect.stringMatching(/content filter edited the caption: cut 1 link/)])
    // The order: the caption is read, as a draft on the morning surface, before anything is uploaded or posted.
    const captionRead = outbound.mock.calls.findIndex(([text]) => text.startsWith('gm builders'))
    expect(outbound.mock.calls[captionRead]?.[1]).toEqual({ surface: 'morning', kind: 'draft', tool: 'morning_post' })
    const uploaded = (x.uploadMedia as ReturnType<typeof vi.fn>).mock.invocationCallOrder[0]!
    expect(outbound.mock.invocationCallOrder[captionRead]).toBeLessThan(uploaded)
  })

  it('asks the judge about the caption and fails closed on its verdict', async () => {
    // The judge sees the scene and the script as prompts first; it is the caption it objects to.
    const judge = vi.fn().mockImplementation(async ({ text }: { text: string }) =>
      text.startsWith('gm builders')
        ? { verdict: 'prohibited', category: 'sexual', reason: 'suggestive' }
        : { verdict: 'clean', reason: '' },
    )
    const { run, x, images, videos } = pipeline({ safety: new SafetyGate({ judge: { judge } }) })
    const result = await run()
    expect(judge).toHaveBeenCalledWith({ text: expect.stringContaining('Tiny sits at a sunrise desk'), direction: 'outbound' })
    expect(judge).toHaveBeenCalledWith({ text: expect.stringContaining('0-4s: static shot'), direction: 'outbound' })
    expect(judge).toHaveBeenCalledWith({ text: expect.stringContaining('gm builders'), direction: 'outbound' })
    expect(images.generate).toHaveBeenCalled()
    expect(videos.generate).toHaveBeenCalled()
    expect(result.outcome).toBe('held')
    expect(x.createPost).not.toHaveBeenCalled()

    const clean = vi.fn().mockResolvedValue({ verdict: 'clean', reason: '' })
    const { run: ok, x: okX } = pipeline({ safety: new SafetyGate({ judge: { judge: clean } }) })
    await expect(ok()).resolves.toMatchObject({ outcome: 'posted' })
    expect(okX.createPost).toHaveBeenCalled()
  })

  it('reads the scene and the script through the filter as prompts before spending a generation, like the tools do', async () => {
    const record = vi.fn().mockResolvedValue(undefined)
    const holds = { record, allowedLinks: vi.fn().mockResolvedValue([]) } as unknown as HoldLog
    // A scene the lexicon refuses: nothing is drawn, so nothing is posted.
    const { run, x, images, videos } = pipeline({
      safety: lexicalGate({ holds }),
      text: stagedModel({ scene: '{"scene": "Tiny cleaning a shotgun at dawn.", "storyCue": "a shotgun", "alt": "Tiny at dawn."}' }),
    })
    const result = await run()
    expect(images.generate).not.toHaveBeenCalled()
    expect(videos.generate).not.toHaveBeenCalled()
    expect(x.createPost).not.toHaveBeenCalled()
    expect(result.outcome).toBe('skipped')
    expect(result.problems).toEqual([
      'the content filter refused the scene (weapons); nothing drawn',
      'nothing to attach, so nothing was posted',
    ])
    expect(record).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: 'prompt',
        context: { surface: 'morning', kind: 'prompt', tool: 'generate_image' },
        category: 'weapons',
      }),
    )
    // No caption hold for a post that was never going out.
    expect(record).toHaveBeenCalledTimes(1)

    // A script the lexicon refuses: the picture goes alone, and the clip's budget is kept.
    const budget = new MediaBudget({ imagesPerDay: 5, videosPerDay: 1 })
    const { run: runScript, x: scriptX, videos: scriptVideos } = pipeline({
      safety: lexicalGate({ holds }),
      budget,
      text: stagedModel({
        script: '{"script": "0-6s: static shot, Tiny loads a pistol at the desk. 6-12s: Tiny waves it at the camera.", "summary": "Tiny waves"}',
      }),
    })
    const scriptResult = await runScript()
    expect(scriptVideos.generate).not.toHaveBeenCalled()
    expect(budget.remaining('video')).toBe(1)
    expect(scriptResult.outcome).toBe('posted')
    expect(scriptResult.attached).toBe('image')
    expect(scriptResult.problems).toEqual(['the content filter refused the script (weapons); posting the picture'])
    expect(scriptX.uploadMedia).toHaveBeenCalledWith({ data: PNG, mimeType: 'image/png' })
    expect(record).toHaveBeenLastCalledWith(
      expect.objectContaining({ context: { surface: 'morning', kind: 'prompt', tool: 'generate_video' } }),
    )
  })

  it('without a filter, posts as before', async () => {
    const { run, x } = pipeline()
    await expect(run()).resolves.toMatchObject({ outcome: 'posted' })
    expect(x.createPost).toHaveBeenCalled()
  })

  describe('with the ranker\'s reader', () => {
    const read = (verdict: 'worthy' | 'flat', fixes: string[] = []): Critique => ({
      verdict,
      score: verdict === 'worthy' ? 3 : 0.2,
      upside: verdict === 'worthy' ? 3 : 0.2,
      cost: 0,
      predicted: { reply: 0, quote: 0, repost: 0, like: 0, shareViaCopyLink: 0, shareViaDm: 0, share: 0, followAuthor: 0, notInterested: 0, muteAuthor: 0, blockAuthor: 0, report: 0 },
      communityReply: 0,
      firstReply: verdict === 'worthy' ? 'mine is a for-loop' : '',
      disagreement: '',
      shareReason: '',
      riskReason: '',
      fixes,
      summary: verdict,
    })
    const reader = (...reads: Array<Critique | Error>): Critic => {
      const queue = [...reads]
      return {
        critique: vi.fn().mockImplementation(async () => {
          const next = queue.shift()
          if (next instanceof Error) throw next
          if (!next) throw new Error('asked once too often')
          return next
        }),
      }
    }

    it('shows it the script and the caption, rewrites each once on a flat read with the fixes in the prompt, and records the last read', async () => {
      const critic = reader(read('flat', ['let the machine sulk']), read('worthy'), read('flat', ['end on the question']), read('worthy'))
      const { run, text, x } = pipeline({ critic })
      const result = await run()
      expect(result.outcome).toBe('posted')
      expect(x.createPost).toHaveBeenCalledTimes(1)
      expect(result.critiques).toEqual({ script: read('worthy'), caption: read('worthy') })

      const asked = (critic.critique as ReturnType<typeof vi.fn>).mock.calls.map(([input]) => input as { kind: string; media?: string; context?: string })
      expect(asked.map((input) => input.kind)).toEqual(['script', 'script', 'caption', 'caption'])
      expect(asked[0]!.context).toContain('An agent runtime hit 40k stars')
      expect(asked[2]!.media).toBe('Tiny sips coffee at sunrise, then raises the mug to say gm')

      const calls = (text.complete as ReturnType<typeof vi.fn>).mock.calls as Array<[{ system: string; prompt: string }]>
      const scripts = calls.filter(([call]) => call.prompt.includes('"script": string'))
      expect(scripts).toHaveLength(2)
      expect(scripts[1]![0].prompt).toContain('Your first draft:')
      expect(scripts[1]![0].prompt).toContain('1. let the machine sulk')
      expect(scripts[1]![0].system).toBe(scripts[0]![0].system)
      const captions = calls.filter(([call]) => call.prompt.includes('"text": string'))
      expect(captions).toHaveLength(2)
      expect(captions[1]![0].prompt).toContain('1. end on the question')
      expect(captions[1]![0].prompt).toContain('Verdict: FLAT')
    })

    it('leaves a worthy draft alone', async () => {
      const critic = reader(read('worthy'), read('worthy'))
      const { run, text } = pipeline({ critic })
      const result = await run()
      expect(critic.critique).toHaveBeenCalledTimes(2)
      expect(result.critiques?.caption?.verdict).toBe('worthy')
      const calls = (text.complete as ReturnType<typeof vi.fn>).mock.calls as Array<[{ prompt: string }]>
      expect(calls.filter(([call]) => call.prompt.includes('"text": string'))).toHaveLength(1)
    })

    it('fails open: a critic that is down is a line in the problems and the first draft goes out', async () => {
      const critic = reader(new Error('502'), new Error('502'))
      const { run, x } = pipeline({ critic })
      const result = await run()
      expect(result.outcome).toBe('posted')
      expect(x.createPost).toHaveBeenCalledTimes(1)
      expect(result.critiques).toBeUndefined()
      expect(result.problems).toEqual(expect.arrayContaining([
        'the critic could not read the script: 502',
        'the critic could not read the caption: 502',
      ]))
    })
  })
})

describe('the morning job', () => {
  function result(overrides: Partial<MorningResult> = {}): MorningResult {
    return {
      stories: STORIES,
      droppedAsPolitical: 1,
      pick: { story: 1, headline: 'h', topic: 'ai', angle: 'a', visual: 'v' },
      story: STORIES[0]!,
      scene: { concepts: ['a gag'], scene: 's', storyCue: 'c', alt: 'alt' },
      caption: 'gm',
      attached: 'video',
      outcome: 'posted',
      postedId: '1',
      problems: [],
      recalled: { history: 'hist', community: '' },
      ...overrides,
    }
  }
  const logger = () => ({ info: vi.fn(), error: vi.fn() })

  it('runs the pipeline once a day and journals the turn as a morning post', async () => {
    const record = vi.fn().mockResolvedValue(undefined)
    const journal = { record } as unknown as Journal
    let now = Date.parse('2026-09-14T08:00:00Z')
    const run = vi.fn().mockResolvedValue(result())
    const job = new MorningJob({ pipeline: { run }, logger: logger(), journal, now: () => now })

    await expect(job.run()).resolves.toMatchObject({ outcome: 'posted' })
    expect(record).toHaveBeenCalledWith(
      expect.objectContaining({
        id: 'morning:2026-09-14',
        trigger: 'morning',
        said: 'gm',
        outcome: 'posted',
        recalled: ['hist'],
        incoming: '2 stories read, 1 dropped as political',
      }),
    )

    now += 60 * 60 * 1_000
    await expect(job.run()).resolves.toBeUndefined()
    expect(run).toHaveBeenCalledTimes(1)
    now += 20 * 60 * 60 * 1_000
    await job.run()
    expect(run).toHaveBeenCalledTimes(2)
    await job.run({ force: true })
    expect(run).toHaveBeenCalledTimes(3)
  })

  it('asks the cadence first, and runs the pipeline under its own kind when it may', async () => {
    const run = vi.fn().mockResolvedValue(result())
    const kinds: Array<string | undefined> = []
    let answer: Awaited<ReturnType<Cadence['claim']>> = { ok: false, reason: 'gap', lastPostId: '5', lastPostedAt: '2026-09-14T05:00:00.000Z', until: '2026-09-14T13:00:00.000Z' }
    const cadence = {
      claim: vi.fn(async () => answer),
      within: vi.fn(async (kind: string, fn: () => Promise<unknown>) => {
        kinds.push(kind)
        return fn()
      }),
    } as unknown as Pick<Cadence, 'claim' | 'within'>
    const job = new MorningJob({ pipeline: { run }, logger: logger(), cadence, now: () => Date.parse('2026-09-14T07:00:00Z') })

    await expect(job.run()).resolves.toBeUndefined()
    expect(run).not.toHaveBeenCalled()
    expect(cadence.claim).toHaveBeenCalledWith('morning')

    answer = { ok: true }
    await expect(job.run()).resolves.toMatchObject({ outcome: 'posted' })
    expect(kinds).toEqual(['morning'])
  })

  it('journals a failure and still throws it', async () => {
    const record = vi.fn().mockResolvedValue(undefined)
    const job = new MorningJob({
      pipeline: { run: vi.fn().mockRejectedValue(new Error('exa is down')) },
      logger: logger(),
      journal: { record } as unknown as Journal,
      now: () => Date.parse('2026-09-14T08:00:00Z'),
    })
    await expect(job.run()).rejects.toThrow('exa is down')
    expect(record).toHaveBeenCalledWith(expect.objectContaining({ outcome: 'failed', outcomeNote: 'exa is down' }))
  })

  it('journals a held morning as held, with the category and without the caption', async () => {
    const record = vi.fn().mockResolvedValue(undefined)
    const job = new MorningJob({
      pipeline: {
        run: vi.fn().mockResolvedValue({
          ...result({ outcome: 'held', caption: '', problems: ['content filter withheld the caption: weapons'] }),
          postedId: undefined,
        }),
      },
      logger: logger(),
      journal: { record } as unknown as Journal,
      now: () => Date.parse('2026-09-14T08:00:00Z'),
    })
    await expect(job.run()).resolves.toMatchObject({ outcome: 'held' })
    expect(record).toHaveBeenCalledWith(
      expect.objectContaining({
        outcome: 'held',
        said: '',
        outcomeNote: 'content filter withheld the caption: weapons',
        declined: ['content filter withheld the caption: weapons'],
      }),
    )
  })
})

describe('morning config', () => {
  const REQUIRED = {
    X_BOT_USERNAME: 'teenydroid',
    X_API_KEY: 'k',
    X_API_SECRET: 's',
    X_ACCESS_TOKEN: 't',
    X_ACCESS_TOKEN_SECRET: 'ts',
    CORTEX_TOKEN: 'c',
    LLM_API_KEY: 'l',
    LLM_MODEL: 'reasoning',
  }

  it('is off unless switched on, posts at 07:00 UTC, and reads three topics', () => {
    const config = loadConfig(REQUIRED)
    expect(config.MORNING_ENABLED).toBe(false)
    expect(config.MORNING_CRON).toBe('0 7 * * *')
    expect(config.MORNING_NEWS_TOPICS).toHaveLength(3)
    expect(config.MORNING_VIDEO_SECONDS).toBe(10)
    expect(loadConfig({ ...REQUIRED, MORNING_NEWS_TOPICS: ' a , ,b' }).MORNING_NEWS_TOPICS).toEqual(['a', 'b'])
    expect(loadConfig({ ...REQUIRED, SERVICE_ROLE: 'morning' }).SERVICE_ROLE).toBe('morning')
  })
})

describe('fitting an image for X', () => {
  it('leaves small stills, GIFs and videos alone, and re-encodes a big still as a JPEG under the limit', async () => {
    const { fitImageForX, X_IMAGE_MAX_BYTES } = await import('../src/media/fit.js')
    const sharp = (await import('sharp')).default
    const small: XMedia = { data: PNG, mimeType: 'image/png' }
    expect(await fitImageForX(small)).toBe(small)
    const gif: XMedia = { data: new Uint8Array(X_IMAGE_MAX_BYTES + 1), mimeType: 'image/gif' }
    expect(await fitImageForX(gif)).toBe(gif)

    // Noise does not compress, so a 2048-square noise PNG is well over 5 MB.
    const noise = Buffer.alloc(2048 * 2048 * 3)
    let state = 0x9e3779b9
    for (let index = 0; index < noise.length; index += 1) {
      state ^= state << 13
      state ^= state >>> 17
      state ^= state << 5
      noise[index] = state & 0xff
    }
    const big = new Uint8Array(await sharp(noise, { raw: { width: 2048, height: 2048, channels: 3 } }).png().toBuffer())
    expect(big.byteLength).toBeGreaterThan(X_IMAGE_MAX_BYTES)
    const fitted = await fitImageForX({ data: big, mimeType: 'image/png' })
    expect(fitted.mimeType).toBe('image/jpeg')
    expect(fitted.data.byteLength).toBeLessThanOrEqual(X_IMAGE_MAX_BYTES)
    expect((await sharp(fitted.data).metadata()).width).toBeLessThanOrEqual(2048)
  }, 30_000)
})
