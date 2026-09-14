import { applyHouseStyle, countEmoji } from '../agent/house-style.js'
import { MAX_POST_LENGTH } from '../agent/tools/x.js'
import type { ReferenceSeed, ReferenceSheet, TextModel } from '../media/reference.js'
import { withSeed } from '../media/reference.js'
import type { MediaBudget, MediaStore } from '../media/store.js'
import type { ImageGenerator, ImageReader, StoredMedia, VideoGenerator } from '../media/types.js'
import type { SafetyGate } from '../safety/gate.js'
import type { Memory, XTools } from '../types.js'
import { gatherNews, renderStories, type NewsSource, type Story } from './news.js'
import {
  askJson,
  captionPrompt,
  captionSchema,
  checkQuestion,
  checkSchema,
  extractJson,
  pickPrompt,
  pickSchema,
  scenePrompt,
  sceneSchema,
  scriptPrompt,
  scriptSchema,
  seedBriefPrompt,
  seedBriefSchema,
  type Pick,
  type Scene,
  type Script,
  type StageContext,
} from './stages.js'

export interface MorningPipelineOptions {
  news: NewsSource
  text: TextModel
  images: ImageGenerator
  /** Absent: the post carries the picture instead of a clip. */
  videos?: VideoGenerator
  /** Eyes, to check the picture is of Tiny before it goes out. Absent, the first draw is trusted. */
  reader?: ImageReader
  reference?: ReferenceSheet
  store: MediaStore
  budget: MediaBudget
  memory: Memory
  x: XTools
  persona: string
  botUsername: string
  dryRun: boolean
  /** Off, nothing is uploaded or posted even when dryRun is off. The rehearsal script's guard. */
  allowPosting?: boolean
  topics: string[]
  newsDays: number
  storiesPerTopic: number
  /** How long a clip is asked for. The script is written to this length. */
  videoSeconds: number
  maxEmoji: number
  random?: () => number
  now?: () => number
  /** Between two attempts at a clip. Defaults to thirty seconds. */
  retryDelayMs?: number
  sleep?: (ms: number) => Promise<void>
  logger?: { info: (object: unknown, message?: string) => void }
  /**
   * The content filter, outbound, the way the calendar job and the media
   * tools have it. The pipeline holds the generators directly rather than
   * through the tools, so it reads the gate itself where the tools would:
   * on the scene before a picture is drawn and on the script before a clip
   * is filmed, before any budget is spent, and on the caption before it is
   * posted. A refused picture is no picture, and so no post; a refused clip
   * means the picture goes alone; a held caption is a missing morning.
   */
  safety?: { outbound: SafetyGate['outbound'] }
}

/**
 * Surplus, behind the ladder's video surface, refuses a prompt over 2000
 * characters, and an image rung is not promised to take more. The sheet is
 * ~1000 of them, so the seed and the scene share the rest.
 */
export const MAX_GENERATION_PROMPT = 2_000

/** How much of the chosen article the scene writer reads. */
const ARTICLE_CHARACTERS = 3_000

/** How many times a picture is drawn before one is accepted, and a clip attempted before the picture goes alone. */
const IMAGE_ATTEMPTS = 2
const VIDEO_ATTEMPTS = 2

export interface MorningResult {
  stories: Story[]
  droppedAsPolitical: number
  pick: Pick
  story?: Story
  seed?: ReferenceSeed
  scene: Scene
  image?: StoredMedia
  script?: Script
  video?: StoredMedia
  caption: string
  attached: 'video' | 'image' | 'none'
  postedId?: string
  /** `held` is the content filter's: the caption was written and refused, and nothing went out. */
  outcome: 'posted' | 'dry-run' | 'skipped' | 'held'
  /** Every stage that went wrong and was worked around. */
  problems: string[]
  recalled: { history: string; community: string }
}

/**
 * The good-morning post, as a fixed sequence rather than an agent.
 *
 * Read the news, choose one story that is not political, draw Tiny inside
 * it from one of the official pictures, write a short script from the
 * picture, film it, caption the lot, post it. Every step is the same every
 * day, and each is a plain model call asked for one object, so the pipeline
 * never spends a video generation on a whim and never forgets to say good
 * morning. An agent would decide the order; here the order is the point.
 *
 * Memory sits on both ends the way it does around the chart post: what was
 * posted before is recalled so the same story is not told twice, and what
 * went out is written back so tomorrow's run can see it.
 *
 * A stage that fails degrades rather than aborts where it can: no video
 * means the picture goes out; no picture means nothing goes out, because
 * the visual is the post. The problems list says what happened.
 */
export class MorningPipeline {
  constructor(private readonly options: MorningPipelineOptions) {}

  async run(): Promise<MorningResult> {
    const { options } = this
    const now = (options.now ?? Date.now)()
    const today = new Date(now).toISOString().slice(0, 10)
    const context: StageContext = { persona: options.persona, botUsername: options.botUsername, today }
    const problems: string[] = []
    const log = (event: string, detail: Record<string, unknown> = {}): void =>
      options.logger?.info(detail, `morning: ${event}`)

    // 1. What we already said, and who we are saying it to.
    const [history, community] = await Promise.all([
      this.recall(
        'What good-morning posts and news stories has this account published recently? For ' +
          'each: the story, the topic, and how it was framed. Also anything the account has ' +
          'already said about AI, crypto or tech news this week.',
      ),
      this.recall(
        'Who are the holders, builders and community members this account talks to? What ' +
          'do they care about, what have they been asking, and what is the mood right now?',
      ),
    ])

    // 2. The news, minus politics.
    const { stories, dropped } = await gatherNews(options.news, {
      topics: options.topics,
      days: options.newsDays,
      perTopic: options.storiesPerTopic,
    })
    log('news gathered', { stories: stories.length, droppedAsPolitical: dropped })

    // 3. Choose the story.
    const pick = await askJson(options.text, {
      ...pickPrompt(context, { stories: renderStories(stories), history }),
      schema: pickSchema,
    })
    const story = pick.story > 0 ? stories[pick.story - 1] : undefined
    if (pick.story > 0 && !story) {
      problems.push(`the model chose story ${pick.story} of ${stories.length}; using its headline alone`)
    }
    if (!story && pick.fallback) pick.angle = `${pick.angle} (${pick.fallback})`
    log('story chosen', { story: pick.story, topic: pick.topic, headline: pick.headline })

    // 4. A seed picture and the scene, then the picture itself. The seed's
    // description is an illustrator's brief of a whole picture, a thousand
    // characters and more; boiled down to what is particular about it, so it
    // fits next to the sheet and the scene inside a generator's prompt.
    const sheet = options.reference ? await options.reference.get() : undefined
    let seed = options.reference ? await options.reference.seed(options.random) : undefined
    if (seed) {
      try {
        const { brief } = await askJson(options.text, { ...seedBriefPrompt({ look: seed.look }), schema: seedBriefSchema })
        seed = { ...seed, look: brief }
      } catch (error) {
        problems.push(`could not brief the seed picture: ${messageOf(error)}; drawing from the sheet alone`)
        seed = undefined
      }
    }
    // The article itself, for the scene: the excerpt is two sentences and a
    // picture built from two sentences is a picture of a desk. Unreadable is
    // fine; the headline and the picked visual carry it.
    const article = story
      ? await options.news.read(story.url, { maxCharacters: ARTICLE_CHARACTERS }).catch(() => '')
      : ''
    const scene = await askJson(options.text, {
      ...scenePrompt({ pick, article, seedLook: seed?.look }),
      schema: sceneSchema,
    })
    log('scene written', { seed: seed?.url, concepts: scene.concepts, cue: scene.storyCue })

    // Drawn, then looked at: an image rung is not bound to the sheet, and one
    // served a purple robot in rehearsal. A picture that fails the look is
    // drawn again once; the second is kept either way, and the problem noted.
    // The scene passes the filter first, as a prompt does in the generate
    // tool: a refused scene is not drawn at all, and no budget is spent.
    let image: StoredMedia | undefined
    const imagePrompt = withSeed(sheet, seed, scene.scene, { maxLength: MAX_GENERATION_PROMPT })
    const sceneRefused = await this.refused(scene.scene, 'generate_image')
    if (sceneRefused) problems.push(`the content filter refused the scene (${sceneRefused}); nothing drawn`)
    for (let attempt = 1; attempt <= IMAGE_ATTEMPTS && !image && !sceneRefused; attempt += 1) {
      if (!options.budget.take('image')) {
        problems.push('the daily image allowance was already spent')
        break
      }
      let candidate: StoredMedia
      try {
        const generated = await options.images.generate({ prompt: imagePrompt, aspect: 'square' })
        candidate = options.store.put({ kind: 'image', prompt: imagePrompt, ...generated })
        log('image generated', { attempt, mediaId: candidate.id, bytes: generated.data.byteLength, path: candidate.path })
      } catch (error) {
        problems.push(`image generation failed (attempt ${attempt}): ${messageOf(error)}`)
        continue
      }
      const verdict = await this.check(candidate, sheet)
      if (verdict.ok || attempt === IMAGE_ATTEMPTS) {
        image = candidate
        if (!verdict.ok) problems.push(`kept a picture the check did not like: ${verdict.reason}`)
      } else {
        problems.push(`redrew the picture: ${verdict.reason}`)
      }
    }

    // 5. The script, then the clip. The script is written even when there
    // is nothing to film it with: it is what the journal records, and a
    // rehearsal without a video key still shows what would have been asked.
    let script: Script | undefined
    let video: StoredMedia | undefined
    if (image) {
      try {
        script = await askJson(options.text, {
          ...scriptPrompt({ pick, scene, seconds: options.videoSeconds }),
          schema: scriptSchema,
        })
        log('script written', { summary: script.summary })
      } catch (error) {
        problems.push(`script failed: ${messageOf(error)}`)
      }
    }
    if (script && !options.videos) {
      problems.push('video generation is off; posting the picture')
    } else if (script && options.videos) {
      const scriptRefused = await this.refused(script.script, 'generate_video')
      if (scriptRefused) {
        problems.push(`the content filter refused the script (${scriptRefused}); posting the picture`)
      } else if (!options.budget.take('video')) {
        problems.push('the daily video allowance was already spent; posting the picture')
      } else {
        // One job from the budget, tried twice: the marketplace answers
        // "no provider could accept this job right now" often enough that
        // a single miss would make most mornings a picture. It also hands
        // the same failed job back for an identical request, so the second
        // attempt waits and carries a different prompt.
        const prompt = withSeed(sheet, seed, script.script, { maxLength: MAX_GENERATION_PROMPT - 12 })
        for (let attempt = 1; attempt <= VIDEO_ATTEMPTS && !video; attempt += 1) {
          if (attempt > 1) await (options.sleep ?? sleep)(options.retryDelayMs ?? 30_000)
          try {
            const generated = await options.videos.generate({
              prompt: attempt === 1 ? prompt : `${prompt}\n\n(take ${attempt})`,
              orientation: 'square',
              seconds: options.videoSeconds,
            })
            video = options.store.put({ kind: 'video', prompt, ...generated })
            log('video generated', { attempt, mediaId: video.id, bytes: generated.data.byteLength, path: video.path })
          } catch (error) {
            problems.push(
              `video generation failed (attempt ${attempt}): ${messageOf(error)}` +
                (attempt === VIDEO_ATTEMPTS ? '; posting the picture' : ''),
            )
          }
        }
      }
    }

    // 6. The caption. It knows something is attached only so it does not
    // narrate it: the clip is the joke, the words are the good morning.
    const attached: MorningResult['attached'] = video ? 'video' : image ? 'image' : 'none'
    let caption = await this.caption(context, { pick, attached, community })

    // The gate, outbound, as the calendar job reads it: the styled caption
    // is what would be posted, so that is what is checked, and what comes
    // back (links cut) is what is posted. Checked before the dry run, so a
    // rehearsal shows the filter's verdict too, and only when there is
    // something to post, so a morning with no picture does not leave a hold
    // for a caption that was never going out. A held caption is a missing
    // morning, which is the right outcome for a post the account should not
    // have written; the picture is not posted bare and tomorrow runs afresh.
    let held: string | undefined
    const outgoing =
      attached === 'none'
        ? undefined
        : await options.safety?.outbound(caption, { surface: 'morning', kind: 'draft', tool: 'morning_post' })
    if (outgoing) {
      if (outgoing.fixes.length > 0) problems.push(`the content filter edited the caption: ${outgoing.fixes.join('; ')}`)
      if (outgoing.decision.ok) {
        caption = outgoing.text
      } else {
        held = outgoing.decision.category
        log('caption held', { category: held, by: outgoing.decision.by, reason: outgoing.decision.reason })
        problems.push(`content filter withheld the caption: ${held}`)
      }
    }

    const base: Omit<MorningResult, 'outcome' | 'postedId'> = {
      stories,
      droppedAsPolitical: dropped,
      pick,
      ...(story ? { story } : {}),
      ...(seed ? { seed } : {}),
      scene,
      ...(image ? { image } : {}),
      ...(script ? { script } : {}),
      ...(video ? { video } : {}),
      caption,
      attached,
      problems,
      recalled: { history, community },
    }

    // 7. Post, or say what would have been posted.
    if (held) return { ...base, caption: '', outcome: 'held' }
    if (attached === 'none') {
      problems.push('nothing to attach, so nothing was posted')
      return { ...base, outcome: 'skipped' }
    }
    const media = video ?? image!
    if (options.dryRun || options.allowPosting === false) {
      await this.capture(now, { ...base, outcome: 'dry-run' })
      return { ...base, outcome: 'dry-run' }
    }
    const uploaded = await options.x.uploadMedia({ data: media.data, mimeType: media.mimeType })
    const posted = await options.x.createPost(caption, { mediaIds: [uploaded.mediaId] })
    log('posted', { id: posted.id, attached })
    const result: MorningResult = { ...base, outcome: 'posted', postedId: posted.id }
    await this.capture(now, result)
    return result
  }

  /**
   * The caption, checked the way `x_post` checks a draft: over the emoji
   * cap or over length is sent back once, and whatever comes back is put
   * through the house style so a stray dash or a fourth emoji is corrected
   * rather than posted.
   */
  private async caption(
    context: StageContext,
    input: {
      pick: Pick
      attached: MorningResult['attached']
      community: string
    },
  ): Promise<string> {
    const { maxEmoji } = this.options
    const ask = captionPrompt(context, { ...input, maxEmoji, maxLength: MAX_POST_LENGTH })
    let { text } = await askJson(this.options.text, { ...ask, schema: captionSchema })
    if (text.length > MAX_POST_LENGTH || countEmoji(text) > maxEmoji) {
      const retry = await askJson(this.options.text, {
        system: ask.system,
        prompt:
          `${ask.prompt}\n\nYour draft was ${text.length} characters with ${countEmoji(text)} ` +
          `emoji:\n${text}\n\nRewrite it under ${MAX_POST_LENGTH} characters with at most ` +
          `${maxEmoji} emoji, keeping the three parts.`,
        schema: captionSchema,
      })
      text = retry.text
    }
    return applyHouseStyle(text, { maxEmoji, maxLength: MAX_POST_LENGTH }).text
  }

  /**
   * The filter's reading of a generation prompt, as the generate tools take
   * it: on the scene or script as written, before the sheet and the seed are
   * put in front of it, and before any budget is spent. Returns the category
   * it was refused for, or nothing when it may be drawn. No gate, no reading.
   */
  private async refused(prompt: string, tool: 'generate_image' | 'generate_video'): Promise<string | undefined> {
    const { safety } = this.options
    if (!safety) return undefined
    const result = await safety.outbound(prompt, { surface: 'morning', kind: 'prompt', tool })
    return result.decision.ok ? undefined : result.decision.category
  }

  /** The vision ladder's verdict on a drawn picture. No eyes, or eyes that fail, means it passes. */
  private async check(image: StoredMedia, sheet: string | undefined): Promise<{ ok: boolean; reason: string }> {
    const { reader } = this.options
    if (!reader || image.mimeType === 'video/mp4') return { ok: true, reason: 'no vision model to check with' }
    try {
      const answer = await reader.describe({ data: image.data, mimeType: image.mimeType }, checkQuestion(sheet))
      const parsed = checkSchema.safeParse(extractJson(answer))
      if (parsed.success) return { ok: parsed.data.ok, reason: parsed.data.reason.slice(0, 300) }
      // Not parseable JSON -- a verdict with an unescaped quote in its reason,
      // or one cut off mid-sentence -- still carries its verdict in the first
      // few characters; then a plain yes or no; fail closed past that.
      const verdict = /"ok"\s*:\s*(true|false)/i.exec(answer)
      const ok = verdict ? verdict[1]!.toLowerCase() === 'true' : /^\s*(yes|true)/i.test(answer)
      return { ok, reason: answer.slice(0, 300) }
    } catch (error) {
      return { ok: true, reason: `could not check: ${messageOf(error)}` }
    }
  }

  private async recall(query: string): Promise<string> {
    return this.options.memory.recall(query).catch(() => '')
  }

  /**
   * What went out, written back so tomorrow's pick can see it. The whole
   * post, the story it was about, and the prompts, so "do not repeat
   * yesterday" is a judgement about the actual text. Dry runs are captured
   * too, labelled, like chart rehearsals are.
   */
  private async capture(now: number, result: MorningResult): Promise<void> {
    const id = result.postedId ?? `dry-run:${now}`
    await this.options.memory
      .captureActivity('morning_post', id, {
        text: result.caption,
        topic: result.pick.topic,
        headline: result.pick.headline,
        angle: result.pick.angle,
        ...(result.story ? { source: result.story.source, url: result.story.url } : {}),
        ...(result.seed ? { seed: result.seed.url } : {}),
        scene: result.scene.scene,
        storyCue: result.scene.storyCue,
        ...(result.script ? { script: result.script.script, clip: result.script.summary } : {}),
        attached: result.attached,
        published: result.outcome === 'posted',
      })
      .catch((error: unknown) => {
        result.problems.push(`could not remember the post: ${messageOf(error)}`)
      })
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
