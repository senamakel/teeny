import { pickEngine } from '../backrooms/context.js'
import type { Thought } from '../backrooms/distill.js'
import { crazinessTotal, renderTurn, type Craziness } from '../backrooms/evaluate.js'
import type { BackroomsRun, Seed } from '../backrooms/types.js'
import type { Backrooms, RunResult } from '../backrooms/wiring.js'
import { renderCritique, type Critic, type Critique } from '../critic/algo.js'
import { withSeed, type ReferenceSheet, type TextModel } from '../media/reference.js'
import type { MediaBudget, MediaStore } from '../media/store.js'
import type { ImageGenerator, ImageReader, StoredMedia } from '../media/types.js'
import { MAX_GENERATION_PROMPT } from '../morning/pipeline.js'
import { askJson, checkQuestion, checkSchema, extractJson, seedBriefPrompt, seedBriefSchema } from '../morning/stages.js'
import type { NarrativeStateStore } from '../narrative/state.js'
import { renderLoreForTeeny, type Lore } from '../narrative/voice.js'
import type { SafetyGate } from '../safety/gate.js'
import type { Memory, XArticleContentState, XTools } from '../types.js'
import { markdownToContentState, type ArticleImageRef } from './content-state.js'
import { proofread, unfence, type ProofResult } from './proofread.js'
import {
  articleStyle,
  assembleMarkdown,
  checkStructurePrompt,
  dekOf,
  normaliseStructure,
  reviseStructurePrompt,
  sectionPrompt,
  stripSectionExtras,
  structureCheckSchema,
  structurePrompt,
  structureSchema,
  tightenPrompt,
  wordCount,
  type ArticleMaterial,
  type Structure,
  type StructureCheck,
} from './stages.js'
import { freeSlug, slugFor, type ArticleImage, type ArticleRecord, type ArticleStore } from './store.js'

const IMAGE_ATTEMPTS = 2

/**
 * One article, as a fixed sequence.
 *
 * The backrooms find the idea: a room is opened on something from the
 * wire, steered at one of the lore's engines, and read down to a thought.
 * The article is then planned before it is written, written one section
 * at a time against that plan, read by the proof-reader with the humanizer
 * skill as the checklist, held to the voice's few hard rules, gated per
 * section, and only then drawn. Every stage's output is written to the
 * record, so a rehearsal can be read stage by stage and nothing goes to X
 * until `publish` is on: a run without it is a dry run that saves
 * everything and posts nothing.
 */
export interface ArticlePipelineOptions {
  backrooms: Pick<Backrooms, 'run'>
  /** A story from the wire on a topic, or nothing. */
  seed: (topic: string) => Promise<Seed | undefined>
  topics: string[]
  /** The raw lore, for the engines, and the parsed lore, for what teeny may say. */
  loreText: string
  lore: Lore
  /** The serial's ledger, read for what has surfaced; never written. */
  state: Pick<NarrativeStateStore, 'load'>
  /** Warm, for the sections; cold, for the plan, the check and the proof-reader's review; the fixer in between. */
  writer: TextModel
  editor: TextModel
  fixer: TextModel
  humanizer: string
  images?: ImageGenerator
  reader?: ImageReader
  reference?: ReferenceSheet
  store: MediaStore
  budget: MediaBudget
  x: Pick<XTools, 'uploadMedia' | 'publishArticle'>
  articles: ArticleStore
  memory?: Memory
  critic?: Critic
  safety?: Pick<SafetyGate, 'outbound'>
  rooms: number
  sectionsMax: number
  words: number
  /** Whether anything goes to X. Off, the run is a rehearsal that saves a `dry-run` record. */
  publish: boolean
  /** Stop after the plan and its check; nothing is expanded. */
  structureOnly?: boolean
  noImages?: boolean
  noProof?: boolean
  /** A room already on disk: skips the seed and the rooms. */
  from?: { run: BackroomsRun; thought: Thought; judge?: Craziness; path?: string }
  topic?: string
  engine?: string
  /** Open the room on this text instead of the wire. */
  seedText?: string
  /** An editorial note the plan, the sections and the check are held to. */
  brief?: string
  random?: () => number
  now?: () => number
  logger?: { info: (object: unknown, message?: string) => void }
}

export interface ArticleRoom {
  index: number
  runId?: string
  total?: number
  rejected?: string
  saved?: string
}

export interface ArticleResult {
  outcome: 'posted' | 'dry-run' | 'structure' | 'held' | 'skipped'
  slug?: string
  engine?: string
  seed?: Seed
  rooms: ArticleRoom[]
  thought?: Thought
  structure?: Structure
  structureCheck?: StructureCheck
  sections: string[]
  markdown: string
  wordCount: number
  proof?: ProofResult
  critique?: Critique
  cover?: StoredMedia
  images: Array<{ key: string; media: StoredMedia }>
  contentState?: XArticleContentState
  xArticleId?: string
  postId?: string
  problems: string[]
}

export class ArticlePipeline {
  constructor(private readonly options: ArticlePipelineOptions) {}

  async run(): Promise<ArticleResult> {
    const { options } = this
    const now = options.now ?? Date.now
    const random = options.random ?? Math.random
    const problems: string[] = []
    const log = (message: string, detail?: Record<string, unknown>): void => options.logger?.info(detail ?? {}, `article: ${message}`)
    const record = new Recorder(options.articles, new Date(now()).toISOString())

    // 1. The lore, and what tonight is about. The serial's ledger says what
    // has surfaced; the article never names the rest.
    const surfaced = (await options.state.load()).surfacedLore
    const lore = renderLoreForTeeny(options.lore, surfaced)
    const engine = options.engine ?? (await this.chooseEngine(random))
    if (engine) record.set({ engine })

    // 2. The seed and 3. the rooms, unless one is handed in.
    let seed: Seed | undefined
    let run: BackroomsRun | undefined
    let thought: Thought | undefined
    let judge: Craziness | undefined
    const rooms: ArticleRoom[] = []
    if (options.from) {
      ;({ run, thought, judge } = options.from)
      seed = run.seed
      rooms.push({ index: 0, runId: run.id, ...(judge ? { total: crazinessTotal(judge) } : {}), ...(options.from.path ? { saved: options.from.path } : {}) })
    } else {
      seed = await this.findSeed(engine, random, problems)
      const results = await Promise.all(
        Array.from({ length: options.rooms }, () =>
          options.backrooms
            .run(seed!, { score: true, distill: true, variants: false, ...(engine ? { engine } : {}), rng: Math.floor(random() * 1_000_000) })
            .then((result): RunResult | { error: string } => result)
            .catch((error: unknown) => ({ error: messageOf(error) })),
        ),
      )
      results.forEach((result, index) => {
        if ('error' in result) {
          rooms.push({ index, rejected: `the room failed: ${result.error}` })
          return
        }
        const total = result.judge ? crazinessTotal(result.judge) : 0
        rooms.push({ index, runId: result.run.id, total, saved: result.saved.json, ...(result.thought ? {} : { rejected: 'no thought came out' }) })
        log('room', { index, runId: result.run.id, total, thought: result.thought?.premise })
      })
      const best = results
        .map((result, index) => ({ result, index }))
        .filter((item): item is { result: RunResult; index: number } => !('error' in item.result) && Boolean(item.result.thought))
        .sort((a, b) => (rooms[b.index]!.total ?? 0) - (rooms[a.index]!.total ?? 0))[0]
      if (best) ({ run, thought, judge } = best.result)
    }
    record.set({ ...(seed ? { seed } : {}), rooms: rooms.map(({ runId, total, rejected }) => ({ ...(runId ? { runId } : {}), ...(total !== undefined ? { total } : {}), ...(rejected ? { rejected } : {}) })) })
    const base = { rooms, sections: [] as string[], markdown: '', wordCount: 0, images: [] as ArticleResult['images'], problems, ...(engine ? { engine } : {}), ...(seed ? { seed } : {}) }
    if (!run || !thought) {
      problems.push('no room produced a thought')
      await record.save('skipped', problems)
      return { ...base, outcome: 'skipped' }
    }
    record.set({ runId: run.id, thought, ...(judge ? { judge } : {}), ...(rooms.find((room) => room.runId === run!.id)?.saved ? { runPath: rooms.find((room) => room.runId === run!.id)!.saved! } : {}) })
    const m: ArticleMaterial = { run, thought, ...(engine ? { engine } : {}), ...(options.brief ? { brief: options.brief } : {}), lore }

    // 4. The plan, checked once before a word of prose exists.
    const planning = { sectionsMax: options.sectionsMax, words: options.words }
    let structure = normaliseStructure(await askJson(options.editor, { ...structurePrompt(m, planning), schema: structureSchema }), options.words)
    let check = await this.checkStructure(structure, m, problems)
    if (check && !check.ok) {
      log('structure revised', { problems: check.problems })
      try {
        structure = normaliseStructure(await askJson(options.editor, { ...reviseStructurePrompt(structure, check, m, planning), schema: structureSchema }), options.words)
        const again = await this.checkStructure(structure, m, problems)
        if (again && !again.ok) problems.push(`the plan still has problems after a revision: ${again.problems.join('; ')}`)
        check = again ?? check
      } catch (error) {
        problems.push(`could not revise the plan: ${messageOf(error)}`)
      }
    }
    record.set({ title: structure.title, dek: structure.dek, structure, ...(check ? { structureCheck: check } : {}) })
    await record.slug(structure.title)
    log('structure', { title: structure.title, sections: structure.sections.map((section) => section.heading) })
    if (options.structureOnly) {
      await record.save('structure', problems)
      return { ...base, outcome: 'structure', slug: record.current.slug, thought, structure, ...(check ? { structureCheck: check } : {}) }
    }

    // 5. The sections, in order, each with the earlier ones in view.
    const bodies: string[] = []
    for (let index = 0; index < structure.sections.length; index += 1) {
      const body = unfence(await options.writer.complete(sectionPrompt(structure, index, bodies, m)))
      if (!body.trim()) throw new Error(`section ${index + 1} came back empty`)
      bodies.push(stripSectionExtras(body))
      log('section', { index, heading: structure.sections[index]!.heading, words: wordCount(bodies[index]!) })
    }

    // 6. Cut down when it ran long.
    let total = bodies.reduce((sum, body) => sum + wordCount(body), 0)
    if (total > options.words * 1.25) {
      const order = structure.sections.map((section, index) => ({ index, over: wordCount(bodies[index]!) - section.words })).sort((a, b) => b.over - a.over)
      for (const { index, over } of order) {
        if (over <= 0 || total <= options.words * 1.1) break
        try {
          const body = unfence(await options.writer.complete(tightenPrompt(structure, index, bodies[index]!, structure.sections[index]!.words)))
          if (body.trim()) bodies[index] = stripSectionExtras(body)
          total = bodies.reduce((sum, item) => sum + wordCount(item), 0)
        } catch (error) {
          problems.push(`could not tighten section ${index + 1}: ${messageOf(error)}`)
        }
      }
    }
    let markdown = assembleMarkdown(structure, bodies)
    record.set({ sections: structure.sections.map((section, index) => ({ key: section.key, heading: section.heading, markdown: bodies[index]! })), markdown, wordCount: wordCount(markdown) })

    // 7. The proof-reader.
    let proof: ProofResult | undefined
    if (!options.noProof) {
      try {
        proof = await proofread({ reviewer: options.editor, fixer: options.fixer }, { skill: options.humanizer, markdown, sample: thought.thought })
        markdown = proof.markdown
        if (proof.refused) problems.push(`the proof-reader's fix was refused: ${proof.refused}`)
        log('proof-read', { verdict: proof.review.verdict, findings: proof.review.findings.length, applied: proof.applied, after: proof.after?.findings.length })
        record.set({ proof: { review: proof.review, applied: proof.applied, ...(proof.refused ? { refused: proof.refused } : {}), ...(proof.after ? { after: proof.after } : {}) } })
      } catch (error) {
        problems.push(`the proof-reader failed: ${messageOf(error)}`)
      }
    }

    // 8. The voice's hard rules, the safety gate per section, and the critic.
    const styled = articleStyle(markdown)
    markdown = styled.text
    if (styled.fixes.length > 0) problems.push(`the style pass ${styled.fixes.join(', ')}`)
    // The proof-reader may have touched the dek; the record and the plan carry what the article says.
    const dek = dekOf(markdown)
    if (dek && dek !== structure.dek) structure = { ...structure, dek }
    record.set({ markdown, wordCount: wordCount(markdown), dek: structure.dek })
    if (styled.priced.length > 0) {
      problems.push(`the article talks money, which it may not: ${styled.priced.join(' | ')}`)
      await record.save('skipped', problems)
      return { ...base, outcome: 'skipped', slug: record.current.slug, thought, structure, sections: bodies, markdown, wordCount: wordCount(markdown), ...(proof ? { proof } : {}) }
    }
    const held = await this.gate(structure, markdown, problems)
    if (held) {
      log('held', { category: held })
      await record.save('held', problems)
      return { ...base, outcome: 'held', slug: record.current.slug, thought, structure, sections: bodies, markdown, wordCount: wordCount(markdown), ...(proof ? { proof } : {}) }
    }
    let critique: Critique | undefined
    if (options.critic) {
      try {
        critique = await options.critic.critique({
          kind: 'other',
          text: markdown.slice(0, 8_000),
          context: `a long-form article from teeny's serial, published as an x article. the premise: ${thought.premise}`,
        })
        record.set({ critique: renderCritique(critique) })
        log('critiqued', { verdict: critique.verdict, score: critique.score })
        if (critique.verdict === 'risky') {
          problems.push(`the critic read it as risky: ${critique.riskReason || critique.summary}`)
          await record.save('skipped', problems)
          return { ...base, outcome: 'skipped', slug: record.current.slug, thought, structure, sections: bodies, markdown, wordCount: wordCount(markdown), ...(proof ? { proof } : {}), critique }
        }
      } catch (error) {
        problems.push(`the critic could not read it: ${messageOf(error)}`)
      }
    }

    // 9. Drawn: the cover and one picture per section, bytes copied into
    // the article's directory as they come.
    const images: ArticleResult['images'] = []
    let cover: StoredMedia | undefined
    const files: Array<{ file: string; data: Uint8Array }> = []
    const kept: ArticleImage[] = []
    if (!options.noImages && options.images) {
      const wanted = [{ key: 'cover', brief: structure.cover }, ...structure.sections.map((section) => ({ key: section.key, brief: section.image }))]
      if (options.budget.remaining('image') < wanted.length) {
        problems.push(`only ${options.budget.remaining('image')} of the day's image allowance is left; drawing the cover first and stopping when it runs out`)
      }
      const sheet = options.reference ? await options.reference.get() : undefined
      let look = options.reference ? await options.reference.seed(random) : undefined
      if (look) {
        try {
          const { brief } = await askJson(options.editor, { ...seedBriefPrompt({ look: look.look }), schema: seedBriefSchema })
          look = { ...look, look: brief }
        } catch (error) {
          problems.push(`could not brief the seed picture: ${messageOf(error)}; drawing from the sheet alone`)
          look = undefined
        }
      }
      for (const { key, brief } of wanted) {
        const media = await this.draw(key, withSeed(sheet, look, brief.brief, { maxLength: MAX_GENERATION_PROMPT }), sheet, problems, log)
        if (!media || media.mimeType === 'video/mp4') continue
        const file = `${key}.${media.mimeType === 'image/jpeg' ? 'jpg' : media.mimeType === 'image/webp' ? 'webp' : 'png'}`
        files.push({ file, data: media.data })
        const image: ArticleImage = { key, file, mimeType: media.mimeType, prompt: media.prompt, caption: brief.caption }
        if (key === 'cover') {
          cover = media
          record.set({ cover: image })
        } else {
          images.push({ key, media })
          kept.push(image)
        }
      }
      if (!cover && images[0]) {
        problems.push('no cover could be drawn; the first section picture stands in')
        cover = images[0].media
        record.set({ cover: { ...kept[0]!, key: 'cover' } })
      }
      record.set({ images: kept })
    } else if (!options.noImages) {
      problems.push('image generation is off; the article goes without pictures')
    }

    // 10. Saved, then 11. published, or not.
    const partial = { ...base, slug: record.current.slug, thought, structure, ...(check ? { structureCheck: check } : {}), sections: bodies, markdown, wordCount: wordCount(markdown), ...(proof ? { proof } : {}), ...(critique ? { critique } : {}), ...(cover ? { cover } : {}), images }
    if (!options.publish) {
      const contentState = markdownToContentState(
        markdown,
        Object.fromEntries(images.map((image) => [image.key, { mediaId: `dry-run:${image.key}`, caption: kept.find((item) => item.key === image.key)?.caption ?? '' }])),
      )
      record.set({ contentState })
      await record.save('dry-run', problems, files)
      log('dry run', { slug: record.current.slug, blocks: contentState.blocks.length })
      return { ...partial, outcome: 'dry-run', contentState }
    }
    await record.save('draft', problems, files)
    let postId: string
    let articleId: string
    try {
      const refs: Record<string, ArticleImageRef> = {}
      let coverMediaId: string | undefined
      if (cover) coverMediaId = (await options.x.uploadMedia({ data: cover.data, mimeType: cover.mimeType })).mediaId
      for (const image of images) {
        const { mediaId } = await options.x.uploadMedia({ data: image.media.data, mimeType: image.media.mimeType })
        refs[image.key] = { mediaId, caption: kept.find((item) => item.key === image.key)?.caption ?? '' }
      }
      record.set({
        ...(record.current.cover && coverMediaId ? { cover: { ...record.current.cover, mediaId: coverMediaId } } : {}),
        images: kept.map((image) => ({ ...image, ...(refs[image.key] ? { mediaId: refs[image.key]!.mediaId } : {}) })),
      })
      const contentState = markdownToContentState(markdown, refs)
      record.set({ contentState })
      ;({ articleId, postId } = await options.x.publishArticle({ title: structure.title, contentState, ...(coverMediaId ? { coverMediaId } : {}) }))
      record.set({ xArticleId: articleId, postId, publishedAt: new Date(now()).toISOString() })
      await record.save('published', problems)
      log('published', { slug: record.current.slug, articleId, postId })
      await options.memory
        ?.captureActivity('article', postId, {
          slug: record.current.slug,
          title: structure.title,
          dek: structure.dek,
          engine,
          premise: thought.premise,
          sections: structure.sections.map((section) => section.heading),
          published: true,
        })
        .catch((error: unknown) => problems.push(`could not remember the article: ${messageOf(error)}`))
      return { ...partial, outcome: 'posted', contentState, xArticleId: articleId, postId }
    } catch (error) {
      record.set({ error: messageOf(error) })
      await record.save('failed', problems)
      throw error
    }
  }

  private async chooseEngine(random: () => number): Promise<string | undefined> {
    const last = await this.options.articles.lastEngine().catch(() => undefined)
    for (let attempt = 0; attempt < 6; attempt += 1) {
      const engine = pickEngine(this.options.loreText, random)
      if (!engine) return undefined
      if (engine !== last) return engine
    }
    return pickEngine(this.options.loreText, random)
  }

  /** From the wire on one of the topics, or as a last resort the engine itself. */
  private async findSeed(engine: string | undefined, random: () => number, problems: string[]): Promise<Seed> {
    const { options } = this
    if (options.seedText) return { kind: 'text', text: options.seedText, source: 'the editor' }
    const topic = options.topic ?? options.topics[Math.floor(random() * options.topics.length)] ?? 'ai agents'
    try {
      const seed = await options.seed(topic)
      if (seed) return seed
      problems.push(`nothing admissible came through the wire for "${topic}"; the room opened on the engine alone`)
    } catch (error) {
      problems.push(`the wire failed for "${topic}": ${messageOf(error)}; the room opened on the engine alone`)
    }
    return { kind: 'text', text: engine ?? 'the lab at night', source: 'lore' }
  }

  private async checkStructure(structure: Structure, m: ArticleMaterial, problems: string[]): Promise<StructureCheck | undefined> {
    try {
      return await askJson(this.options.editor, { ...checkStructurePrompt(structure, m), schema: structureCheckSchema })
    } catch (error) {
      problems.push(`the plan could not be checked: ${messageOf(error)}`)
      return undefined
    }
  }

  /**
   * The title and dek once, then each section on its own: the judge reads
   * at most a few thousand characters, and an article is more than that.
   * The category of the first hold, or nothing.
   */
  private async gate(structure: Structure, markdown: string, problems: string[]): Promise<string | undefined> {
    const { safety } = this.options
    if (!safety) return undefined
    const pieces = [`${structure.title}\n${structure.dek}`, ...splitSections(markdown)]
    for (const piece of pieces) {
      const result = await safety.outbound(piece, { surface: 'article', kind: 'draft', tool: 'article_publish' })
      if (result.fixes.length > 0) problems.push(`the content filter would edit the article: ${result.fixes.join('; ')}`)
      if (!result.decision.ok) {
        problems.push(`content filter withheld the article: ${result.decision.category} (${result.decision.reason})`)
        return result.decision.category
      }
    }
    return undefined
  }

  private async draw(
    key: string,
    prompt: string,
    sheet: string | undefined,
    problems: string[],
    log: (message: string, detail?: Record<string, unknown>) => void,
  ): Promise<StoredMedia | undefined> {
    const { options } = this
    if (!options.images) return undefined
    if (options.safety) {
      const refused = await options.safety.outbound(prompt, { surface: 'article', kind: 'prompt', tool: 'generate_image' })
      if (!refused.decision.ok) {
        problems.push(`the content filter refused the ${key} picture (${refused.decision.category})`)
        return undefined
      }
    }
    let image: StoredMedia | undefined
    for (let attempt = 1; attempt <= IMAGE_ATTEMPTS && !image; attempt += 1) {
      if (!options.budget.take('image')) {
        problems.push(`the daily image allowance ran out before the ${key} picture`)
        return undefined
      }
      let candidate: StoredMedia
      try {
        const generated = await options.images.generate({ prompt, aspect: 'landscape' })
        candidate = options.store.put({ kind: 'image', prompt, ...generated })
        log('image generated', { key, attempt, mediaId: candidate.id })
      } catch (error) {
        problems.push(`the ${key} picture failed (attempt ${attempt}): ${messageOf(error)}`)
        continue
      }
      const verdict = await this.check(candidate, sheet)
      if (verdict.ok || attempt === IMAGE_ATTEMPTS) {
        image = candidate
        if (!verdict.ok) problems.push(`kept a ${key} picture the check did not like: ${verdict.reason}`)
      } else {
        problems.push(`redrew the ${key} picture: ${verdict.reason}`)
      }
    }
    return image
  }

  /** The morning's check: is the small yellow robot in the picture Tiny. */
  private async check(image: StoredMedia, sheet: string | undefined): Promise<{ ok: boolean; reason: string }> {
    const { reader } = this.options
    if (!reader || image.mimeType === 'video/mp4') return { ok: true, reason: 'no vision model to check with' }
    try {
      const answer = await reader.describe({ data: image.data, mimeType: image.mimeType }, checkQuestion(sheet))
      const parsed = checkSchema.safeParse(extractJson(answer))
      if (parsed.success) return { ok: parsed.data.ok, reason: parsed.data.reason.slice(0, 300) }
      const verdict = /"ok"\s*:\s*(true|false)/i.exec(answer)
      const ok = verdict ? verdict[1]!.toLowerCase() === 'true' : /^\s*(yes|true)/i.test(answer)
      return { ok, reason: answer.slice(0, 300) }
    } catch (error) {
      return { ok: true, reason: `could not check: ${messageOf(error)}` }
    }
  }
}

/** The record as it grows, written at every stop so a crash leaves what was done. */
class Recorder {
  current: ArticleRecord

  constructor(
    private readonly articles: ArticleStore,
    createdAt: string,
  ) {
    this.current = { slug: `pending-${createdAt.replace(/[^0-9]/g, '').slice(0, 14)}`, title: '', dek: '', status: 'structure', createdAt, rooms: [], sections: [], markdown: '', wordCount: 0, images: [], problems: [] }
  }

  set(fields: Partial<ArticleRecord>): void {
    this.current = { ...this.current, ...fields }
  }

  async slug(title: string): Promise<void> {
    this.current.slug = await freeSlug(this.articles, slugFor(new Date(this.current.createdAt), title))
  }

  async save(status: ArticleRecord['status'], problems: string[], files: Array<{ file: string; data: Uint8Array }> = []): Promise<void> {
    this.current = { ...this.current, status, problems: [...problems] }
    await this.articles.save(this.current, files)
  }
}

/** The dek, then each `## ` section with its heading, as separate pieces. */
export function splitSections(markdown: string): string[] {
  const pieces: string[] = []
  let current: string[] = []
  for (const line of markdown.split('\n')) {
    if (/^##\s/.test(line) && current.length > 0) {
      pieces.push(current.join('\n').trim())
      current = []
    }
    current.push(line)
  }
  if (current.length > 0) pieces.push(current.join('\n').trim())
  return pieces.filter((piece) => piece.length > 0)
}

export function renderRoom(run: BackroomsRun): string {
  return run.turns.map((turn, index) => renderTurn(turn, index)).join('\n\n')
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
