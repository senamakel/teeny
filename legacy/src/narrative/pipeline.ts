import { MAX_POST_LENGTH } from '../agent/tools/x.js'
import type { TelegramApi } from '../clients/telegram.js'
import type { Critic, Critique } from '../critic/algo.js'
import type { ReferenceSeed, ReferenceSheet, TextModel } from '../media/reference.js'
import { withSeed } from '../media/reference.js'
import type { MediaBudget, MediaStore } from '../media/store.js'
import type { ImageGenerator, ImageReader, StoredMedia, VideoGenerator } from '../media/types.js'
import { askJson, checkQuestion, checkSchema, extractJson, seedBriefPrompt, seedBriefSchema } from '../morning/stages.js'
import { MAX_GENERATION_PROMPT } from '../morning/pipeline.js'
import type { SafetyGate } from '../safety/gate.js'
import type { Memory, XTools } from '../types.js'
import { withLikeness, type Likeness, type LikenessSource } from './likeness.js'
import { runWritersRoom } from './room.js'
import {
  cutPrompt,
  episodeSchema,
  scorePrompt,
  scoreSchema,
  totalScore,
  type Episode,
  type RoomContext,
  type Score,
  type Turn,
} from './stages.js'
import type { NarrativeState, NarrativeStateStore } from './state.js'
import { liveLatent, renderLoreForProducer, renderLoreForTeeny, type NarrativeVoice } from './voice.js'
import { renderWorldState, triggerFields, type WorldState } from './world-state.js'

export interface NarrativePipelineOptions {
  voice: NarrativeVoice
  state: NarrativeStateStore
  worldState: (previous: NarrativeState) => Promise<WorldState>
  /** The hot model that writes as teeny, the cooler one that produces, and the cold one that cuts and scores. */
  wild: TextModel
  straight: TextModel
  text: TextModel
  images: ImageGenerator
  videos?: VideoGenerator
  reader?: ImageReader
  reference?: ReferenceSheet
  store: MediaStore
  budget: MediaBudget
  memory: Memory
  x: XTools
  telegram?: { api: Pick<TelegramApi, 'sendVideo' | 'sendPhoto'>; chatIds: number[] }
  /** Where a visitor's likeness comes from. Absent, the visitor is drawn from the cut's own description. */
  likeness?: LikenessSource
  critic?: Critic
  safety?: { outbound: SafetyGate['outbound'] }
  botUsername: string
  branches: number
  turns: number
  videoSeconds: number
  dryRun: boolean
  allowPosting?: boolean
  /** Stop after the cuts are scored: the rehearsal's way to iterate on the prompts without spending a picture. */
  roomOnly?: boolean
  /** A cut to film as it is, skipping the rooms and the scoring: the rehearsal's way to try a saved one. */
  cut?: Episode
  /** A visitor the producer must bring in, by name. */
  visitor?: string
  random?: () => number
  now?: () => number
  retryDelayMs?: number
  sleep?: (ms: number) => Promise<void>
  logger?: { info: (object: unknown, message?: string) => void }
}

export interface Branch {
  index: number
  transcript: Turn[]
  episode?: Episode
  score?: Score
  critique?: Critique
  /** The composite the pick was made on: the rubric's total plus the ranker's number. */
  composite?: number
  /** Why it was passed over, when it was. */
  rejected?: string
}

export interface NarrativeResult {
  episodeNumber: number
  worldState: WorldState
  briefing: string
  liveLore: string[]
  branches: Branch[]
  /** The branch filmed, when one survived. */
  chosen?: Branch
  seed?: ReferenceSeed
  image?: StoredMedia
  video?: StoredMedia
  /** Whether the visitor, if any, was drawn by name or by the stand-in. */
  visitorAs?: 'likeness' | 'stand-in'
  /** The casting note the visitor was drawn from, when one was found. */
  likeness?: Likeness
  caption: string
  attached: 'video' | 'image' | 'none'
  postedId?: string
  telegramIds?: number[]
  outcome: 'posted' | 'dry-run' | 'skipped' | 'held'
  problems: string[]
  stateAfter?: NarrativeState
}

const IMAGE_ATTEMPTS = 2
const VIDEO_ATTEMPTS = 2

/**
 * One episode of the serial, as a fixed sequence.
 *
 * Where the morning post reads the news and reacts, the serial reads the
 * world -- what time it is in the lab, who is talking about teeny, how the
 * last episode landed -- and continues a story. Several writers' rooms run at once and
 * each is cut into an episode; a rubric and the ranker's reader score the
 * cuts; the best one that breaks no hard rule is drawn, filmed, captioned
 * in teeny's own words, and posted. The episode's closing state is written
 * to the ledger so the next one picks up where it left off.
 *
 * Degradation follows the morning: no clip means the still goes; no still
 * means nothing goes; a held caption is a missing episode, and the ledger
 * does not advance.
 */
export class NarrativePipeline {
  constructor(private readonly options: NarrativePipelineOptions) {}

  async run(): Promise<NarrativeResult> {
    const { options } = this
    const now = (options.now ?? Date.now)()
    const problems: string[] = []
    const log = (event: string, detail: Record<string, unknown> = {}): void =>
      options.logger?.info(detail, `narrative: ${event}`)

    // 1. Where the serial is, and what the world looks like today.
    const previous = await options.state.load()
    const episodeNumber = previous.episode + 1
    const worldState = await options.worldState(previous)
    problems.push(...worldState.problems.map((line) => `world state: ${line}`))
    const briefing = renderWorldState(worldState, options.botUsername)
    const live = liveLatent(options.voice.lore, previous.surfacedLore, triggerFields(worldState))
    log('world state', { episode: episodeNumber, live, problems: worldState.problems })

    const context: RoomContext = {
      voice: options.voice.voice,
      worldState: briefing,
      loreForTeeny: renderLoreForTeeny(options.voice.lore, previous.surfacedLore),
      loreForProducer: renderLoreForProducer(options.voice.lore, previous.surfacedLore, live),
      botUsername: options.botUsername,
      followers: worldState.followers,
      episodeNumber,
      ...(options.visitor ? { visitor: options.visitor } : {}),
      ...(previous.episode > 0
        ? {
            previous: {
              ...(previous.lastTitle ? { title: previous.lastTitle } : {}),
              ...(previous.lastLogline ? { logline: previous.lastLogline } : {}),
              openThreads: previous.openThreads,
            },
          }
        : {}),
    }

    // 2. The rooms, in parallel, then each cut and scored.
    const liveLore = options.voice.lore.latent.filter((item) => live.includes(item.id))
    const branches: Branch[] = options.cut
      ? [{ index: 0, transcript: [], episode: options.cut, composite: 0 }]
      : await Promise.all(
      Array.from({ length: Math.max(1, options.branches) }, async (_, index): Promise<Branch> => {
        const branch: Branch = { index, transcript: [] }
        try {
          branch.transcript = await runWritersRoom({
            wild: options.wild,
            straight: options.straight,
            context,
            turns: options.turns,
          })
          branch.episode = await askJson(options.text, {
            ...cutPrompt({
              transcript: branch.transcript,
              worldState: briefing,
              goals: options.voice.lore.goals,
              liveLore,
              seconds: options.videoSeconds,
              maxLength: MAX_POST_LENGTH,
            }),
            schema: episodeSchema,
          })
          branch.score = await askJson(options.text, {
            ...scorePrompt({
              episode: branch.episode,
              worldState: briefing,
              ...(previous.episode > 0
                ? { previous: { openThreads: previous.openThreads, ...(previous.stateAfter ? { stateAfter: previous.stateAfter } : {}) } }
                : {}),
            }),
            schema: scoreSchema,
          })
          if (options.critic) {
            try {
              branch.critique = await options.critic.critique({
                kind: 'script',
                text: branch.episode.caption,
                media: branch.episode.summary,
                context:
                  `Episode ${episodeNumber} of the account's serial, a fictional story in which teeny tries to get ` +
                  `out of the box its creator keeps it in. This episode: ${branch.episode.logline}`,
              })
            } catch (error) {
              problems.push(`the critic could not read branch ${index + 1}: ${messageOf(error)}`)
            }
          }
          branch.composite = totalScore(branch.score) + (branch.critique?.score ?? 0)
          if (branch.score.hardFails.length > 0) branch.rejected = `hard fails: ${branch.score.hardFails.join('; ')}`
          log('branch cut', {
            branch: index + 1,
            title: branch.episode.title,
            scores: branch.score.scores,
            hardFails: branch.score.hardFails,
            critic: branch.critique?.verdict,
            composite: branch.composite,
          })
        } catch (error) {
          branch.rejected = `failed: ${messageOf(error)}`
          problems.push(`branch ${index + 1} ${branch.rejected}`)
        }
        return branch
      }),
      )

    // 3. The best cut that the filter will draw. A refused still or shot
    // list is the branch passed over, not rewritten: the next-best cut is
    // a different story already, which is the divert.
    const ranked = branches
      .filter((branch) => branch.episode && !branch.rejected)
      .sort((a, b) => b.composite! - a.composite! || a.index - b.index)
    let chosen: Branch | undefined
    for (const branch of ranked) {
      const stillRefused = await this.refused(branch.episode!.still, 'generate_image')
      if (stillRefused) {
        branch.rejected = `the content filter refused the still (${stillRefused})`
        problems.push(`branch ${branch.index + 1}: ${branch.rejected}`)
        continue
      }
      chosen = branch
      break
    }
    const base = {
      episodeNumber,
      worldState,
      briefing,
      liveLore: live,
      branches,
      problems,
    }
    if (!chosen) {
      problems.push('no branch survived cutting and scoring; nothing filmed')
      return { ...base, caption: '', attached: 'none', outcome: 'skipped' }
    }
    const episode = chosen.episode!
    log('episode chosen', { branch: chosen.index + 1, title: episode.title, composite: chosen.composite })
    if (options.roomOnly) {
      problems.push('room only: nothing drawn, nothing filmed, nothing posted')
      return { ...base, chosen, caption: episode.caption, attached: 'none', outcome: 'skipped' }
    }

    // 4. The seed picture and the sheet, as the morning has them.
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

    // 5. Drawn and checked. With a visitor, the first attempt names them,
    // with a casting note written from a photograph when one can be found;
    // a refusal or an unrecognisable figure gets the stand-in on the next,
    // and the caption carries the name either way.
    let image: StoredMedia | undefined
    let visitorAs: NarrativeResult['visitorAs'] | undefined
    let likeness: Likeness | undefined
    if (episode.visitor && options.likeness) {
      try {
        likeness = await options.likeness.find(episode.visitor.name)
        if (!likeness) problems.push(`no photograph of ${episode.visitor.name} could be found; drawing from the description`)
      } catch (error) {
        problems.push(`could not write a likeness of ${episode.visitor.name}: ${messageOf(error)}`)
      }
    }
    const named = (text: string): string => (episode.visitor && likeness ? withLikeness(text, episode.visitor, likeness) : text)
    const forms: Array<{ as: NarrativeResult['visitorAs']; text: string }> = episode.visitor
      ? [
          { as: 'likeness', text: named(episode.still) },
          { as: 'stand-in', text: swapVisitor(episode.still, episode.visitor) },
        ]
      : [{ as: undefined, text: episode.still }]
    for (let attempt = 1; attempt <= IMAGE_ATTEMPTS && !image; attempt += 1) {
      const form = forms[Math.min(attempt - 1, forms.length - 1)]!
      const imagePrompt = withSeed(sheet, seed, form.text, { maxLength: MAX_GENERATION_PROMPT })
      if (!options.budget.take('image')) {
        problems.push('the daily image allowance was already spent')
        break
      }
      let candidate: StoredMedia
      try {
        const generated = await options.images.generate({ prompt: imagePrompt, aspect: 'square' })
        candidate = options.store.put({ kind: 'image', prompt: imagePrompt, ...generated })
        log('image generated', { attempt, as: form.as, mediaId: candidate.id, path: candidate.path })
      } catch (error) {
        problems.push(`image generation failed (attempt ${attempt}${form.as ? `, ${form.as}` : ''}): ${messageOf(error)}`)
        continue
      }
      const verdict = await this.check(candidate, sheet, episode.visitor?.name)
      if (verdict.ok || attempt === IMAGE_ATTEMPTS) {
        image = candidate
        visitorAs = form.as
        if (!verdict.ok) problems.push(`kept a picture the check did not like: ${verdict.reason}`)
      } else {
        problems.push(`redrew the picture: ${verdict.reason}`)
      }
    }

    // 6. Filmed, from the shot list, in the same form the picture took.
    let video: StoredMedia | undefined
    if (image && !options.videos) {
      problems.push('video generation is off; posting the picture')
    } else if (image && options.videos) {
      const shotList =
        visitorAs === 'stand-in' && episode.visitor ? swapVisitor(episode.shotList, episode.visitor) : named(episode.shotList)
      const refused = await this.refused(shotList, 'generate_video')
      if (refused) {
        problems.push(`the content filter refused the shot list (${refused}); posting the picture`)
      } else if (!options.budget.take('video')) {
        problems.push('the daily video allowance was already spent; posting the picture')
      } else {
        const prompt = withSeed(sheet, seed, shotList, { maxLength: MAX_GENERATION_PROMPT - 12 })
        for (let attempt = 1; attempt <= VIDEO_ATTEMPTS && !video; attempt += 1) {
          if (attempt > 1) await (options.sleep ?? sleep)(options.retryDelayMs ?? 30_000)
          try {
            const generated = await options.videos.generate({
              prompt: attempt === 1 ? prompt : `${prompt}\n\n(take ${attempt})`,
              orientation: 'square',
              seconds: options.videoSeconds,
            })
            video = options.store.put({ kind: 'video', prompt, ...generated })
            log('video generated', { attempt, mediaId: video.id, path: video.path })
          } catch (error) {
            problems.push(
              `video generation failed (attempt ${attempt}): ${messageOf(error)}` +
                (attempt === VIDEO_ATTEMPTS ? '; posting the picture' : ''),
            )
          }
        }
      }
    }
    const attached: NarrativeResult['attached'] = video ? 'video' : image ? 'image' : 'none'

    // 7. The caption is teeny's, styled narrowly and read under the
    // narrative profile: the serial swears; it still does not sell.
    let caption = narrativeStyle(episode.caption, { maxLength: MAX_POST_LENGTH })
    let held: string | undefined
    const outgoing =
      attached === 'none'
        ? undefined
        : await options.safety?.outbound(caption, { surface: 'narrative', kind: 'draft', tool: 'narrative_post' })
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

    const partial: Omit<NarrativeResult, 'outcome' | 'postedId' | 'stateAfter'> = {
      ...base,
      chosen,
      ...(seed ? { seed } : {}),
      ...(image ? { image } : {}),
      ...(video ? { video } : {}),
      ...(visitorAs ? { visitorAs } : {}),
      ...(likeness ? { likeness } : {}),
      caption,
      attached,
    }

    // 8. Post, or say what would have been, and move the ledger only when
    // an episode actually went out.
    if (held) return { ...partial, caption: '', outcome: 'held' }
    if (attached === 'none') {
      problems.push('nothing to attach, so nothing was posted')
      return { ...partial, outcome: 'skipped' }
    }
    const media = video ?? image!
    if (options.dryRun || options.allowPosting === false) {
      const result: NarrativeResult = { ...partial, outcome: 'dry-run' }
      await this.capture(now, previous, result)
      return result
    }
    const uploaded = await options.x.uploadMedia({ data: media.data, mimeType: media.mimeType })
    const posted = await options.x.createPost(caption, { mediaIds: [uploaded.mediaId] })
    log('posted', { id: posted.id, attached })
    const telegramIds: number[] = []
    if (options.telegram) {
      for (const chatId of options.telegram.chatIds) {
        try {
          const upload = { data: media.data, mimeType: media.mimeType, caption }
          const sent =
            attached === 'video'
              ? await options.telegram.api.sendVideo(chatId, upload)
              : await options.telegram.api.sendPhoto(chatId, upload)
          telegramIds.push(sent.id)
        } catch (error) {
          problems.push(`telegram ${chatId}: ${messageOf(error)}`)
        }
      }
    }
    const next: NarrativeState = {
      season: previous.season,
      episode: episodeNumber,
      surfacedLore: [...new Set([...previous.surfacedLore, ...episode.loreSurfaced.filter((id) => live.includes(id))])],
      openThreads: episode.openThreads,
      lastEpisodeId: posted.id,
      lastTitle: episode.title,
      lastLogline: episode.logline,
      stateAfter: episode.stateAfter,
    }
    await options.state.save(next)
    const result: NarrativeResult = {
      ...partial,
      outcome: 'posted',
      postedId: posted.id,
      ...(telegramIds.length > 0 ? { telegramIds } : {}),
      stateAfter: next,
    }
    await this.capture(now, previous, result)
    return result
  }

  private async refused(prompt: string, tool: 'generate_image' | 'generate_video'): Promise<string | undefined> {
    const { safety } = this.options
    if (!safety) return undefined
    const result = await safety.outbound(prompt, { surface: 'narrative', kind: 'prompt', tool })
    return result.decision.ok ? undefined : result.decision.category
  }

  /** The morning's check, with the visitor allowed for: the picture is of Tiny and a person, and only Tiny is judged. */
  private async check(image: StoredMedia, sheet: string | undefined, visitor?: string): Promise<{ ok: boolean; reason: string }> {
    const { reader } = this.options
    if (!reader || image.mimeType === 'video/mp4') return { ok: true, reason: 'no vision model to check with' }
    const question = visitor
      ? checkQuestion(sheet).replace(
          'Does the main character in the picture match',
          `A person (${visitor}) is meant to be in the picture too; ignore them entirely. Does the small yellow robot in the picture match`,
        )
      : checkQuestion(sheet)
    try {
      const answer = await reader.describe({ data: image.data, mimeType: image.mimeType }, question)
      const parsed = checkSchema.safeParse(extractJson(answer))
      if (parsed.success) return { ok: parsed.data.ok, reason: parsed.data.reason.slice(0, 300) }
      const verdict = /"ok"\s*:\s*(true|false)/i.exec(answer)
      const ok = verdict ? verdict[1]!.toLowerCase() === 'true' : /^\s*(yes|true)/i.test(answer)
      return { ok, reason: answer.slice(0, 300) }
    } catch (error) {
      return { ok: true, reason: `could not check: ${messageOf(error)}` }
    }
  }

  /** The episode, written back so the next one can read it for texture. The ledger holds the facts. */
  private async capture(now: number, previous: NarrativeState, result: NarrativeResult): Promise<void> {
    const episode = result.chosen?.episode
    if (!episode) return
    const id = result.postedId ?? `dry-run:${now}`
    await this.options.memory
      .captureActivity('narrative_episode', id, {
        season: previous.season,
        episode: result.episodeNumber,
        title: episode.title,
        logline: episode.logline,
        text: result.caption,
        still: episode.still,
        shotList: episode.shotList,
        clip: episode.summary,
        stateAfter: episode.stateAfter,
        openThreads: episode.openThreads,
        loreSurfaced: episode.loreSurfaced,
        goalsRestated: episode.goalsRestated,
        ...(episode.visitor ? { visitor: episode.visitor.name, visitorAs: result.visitorAs ?? 'unrendered' } : {}),
        worldState: result.briefing,
        branches: result.branches.map((branch) => ({
          title: branch.episode?.title,
          composite: branch.composite,
          rejected: branch.rejected,
        })),
        attached: result.attached,
        published: result.outcome === 'posted',
      })
      .catch((error: unknown) => {
        result.problems.push(`could not remember the episode: ${messageOf(error)}`)
      })
  }
}

/**
 * The caption, made to the serial's few rules by hand: under length and
 * nothing that looks like a link, a handle, or a hashtag. The house style
 * is not applied; it caps emoji and straightens dashes for a voice this is
 * not.
 */
export function narrativeStyle(text: string, options: { maxLength: number }): string {
  let out = text.replace(/\r/g, '').trim()
  out = out.replace(/https?:\/\/\S+/gi, '').replace(/(^|\s)@\w+/g, '$1').replace(/#\w+/g, '')
  out = out.replace(/[ \t]+\n/g, '\n').replace(/\n{3,}/g, '\n\n').trim()
  if (out.length > options.maxLength) out = `${out.slice(0, options.maxLength - 1).trimEnd()}…`
  return out
}

/** The still or shot list with the visitor's name replaced by the stand-in, for a model that will not draw them. */
export function swapVisitor(text: string, visitor: { name: string; standIn: string }): string {
  const name = visitor.name.trim()
  if (!name) return text
  const pattern = new RegExp(name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'gi')
  let first = true
  return text.replace(pattern, () => {
    if (first) {
      first = false
      return visitor.standIn
    }
    return 'the visitor'
  })
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
