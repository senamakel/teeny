import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { applyHouseStyle, countEmoji } from '../agent/house-style.js'
import { MAX_POST_LENGTH } from '../agent/tools/x.js'
import { svgToPng } from '../chart/png.js'
import type { TextModel } from '../media/reference.js'
import { askJson } from '../morning/stages.js'
import type { SafetyGate } from '../safety/gate.js'
import type { Memory, XTools } from '../types.js'
import { ladder as buildLadder, pickTarget, rung, usedSymbols, type Rung } from './ladder.js'
import type { LeaderboardSource, ListedCoin } from './leaderboard.js'
import { moonCaptionPrompt, moonCaptionSchema } from './prompt.js'
import { renderMoonSvg, type Logo } from './svg.js'

export interface MoonPipelineOptions {
  source: LeaderboardSource
  text: TextModel
  memory: Memory
  x: XTools
  persona: string
  botUsername: string
  dryRun: boolean
  /** Off, nothing is uploaded or posted even when dryRun is off. The rehearsal script's guard. */
  allowPosting?: boolean
  /** The account's own token, by CoinGecko id. */
  coinId: string
  /** The CoinGecko category the targets come from. */
  category: string
  /** How the category is named on the picture and in the caption. */
  boardName: string
  /** How many of the category's coins are read, largest first. */
  boardSize: number
  /** How many rungs are drawn beside the target. */
  rungs: number
  /**
   * CoinGecko ids never used as a yardstick or a rung: coins the category
   * tag is generous about. They keep their place on the board for the
   * ranks; they are just never the comparison.
   */
  excludeIds?: string[]
  minMultiple?: number
  maxMultiple?: number
  maxEmoji: number
  /** Where a copy of every picture is written. Unset, none is. */
  outputDir?: string
  watermark?: string
  random?: () => number
  now?: () => number
  logger?: { info: (object: unknown, message?: string) => void }
  /** The content filter, on the caption before it is posted. */
  safety?: { outbound: SafetyGate['outbound'] }
}

export interface MoonResult {
  token: ListedCoin
  board: ListedCoin[]
  target: Rung
  ladder: Rung[]
  /** Symbols memory says were posted lately and were therefore skipped. */
  excluded: string[]
  svg: string
  png: Buffer
  /** Where the PNG was written, when an output directory is set. */
  path?: string
  caption: string
  postedId?: string
  outcome: 'posted' | 'dry-run' | 'skipped' | 'held'
  problems: string[]
  recalled: { history: string; community: string }
}

/**
 * "$TINY with the market cap of X", once a day.
 *
 * A fixed sequence, like the morning post, because nothing in it wants a
 * model's judgement about whether to run: read the board and the token,
 * pick a target that has not been used lately, draw the picture from the
 * arithmetic, write two lines under it, post. The picture is drawn here
 * rather than generated, so every number on it is a number this service
 * computed from CoinGecko's row a minute ago, and no generation budget is
 * spent.
 *
 * Memory on both sides, as the other posting jobs have it: what the series
 * already said is recalled so a target is not repeated tomorrow and a joke
 * is not told twice, and what went out is written back with the target's
 * symbol, which is what tomorrow's exclusion list is read from.
 */
export class MoonPipeline {
  constructor(private readonly options: MoonPipelineOptions) {}

  async run(): Promise<MoonResult> {
    const { options } = this
    const now = (options.now ?? Date.now)()
    const today = new Date(now).toISOString().slice(0, 10)
    const problems: string[] = []
    const log = (event: string, detail: Record<string, unknown> = {}): void =>
      options.logger?.info(detail, `moon: ${event}`)

    // 1. What this series has already said, and who it is for.
    const [history, community] = await Promise.all([
      this.recall(
        `Which "market cap of" moon-chart posts has this account published lately? For each: ` +
          'which coin was the yardstick (its ticker), the multiple, and the joke or framing it used.',
      ),
      this.recall(
        'Who are the holders, builders and community members this account talks to? What ' +
          'do they care about, what have they been asking, and what is the mood right now?',
      ),
    ])

    // 2. The board, with our own row on it. The point of the post is where
    // we stand among the AI memes, so the token is looked for on the board
    // first and its position kept; only when the tag has gone missing is
    // the row fetched on its own, and the day says so.
    const board = await options.source.category(options.category, options.boardSize)
    let token = board.find((coin) => coin.id === options.coinId)
    if (!token) {
      token = await options.source.coin(options.coinId)
      problems.push(`$${token.symbol} is not on the ${options.boardName} board (${board.length} coins); compared against it from outside`)
    }
    log('board read', { token: token.symbol, boardRank: token.boardRank, marketCapUsd: token.marketCapUsd, coins: board.length })

    // 3. The target: above us on the board, in the band, not one memory says
    // was used, and not one the operator struck off.
    const struck = new Set(options.excludeIds ?? [])
    const eligible = board.filter((coin) => !struck.has(coin.id) && printable(coin.symbol))
    const exclude = usedSymbols(history, eligible)
    const target = pickTarget(token, eligible, {
      exclude,
      ...(options.minMultiple === undefined ? {} : { minMultiple: options.minMultiple }),
      ...(options.maxMultiple === undefined ? {} : { maxMultiple: options.maxMultiple }),
      ...(options.random ? { random: options.random } : {}),
    })
    if (!target) {
      throw new Error(`nothing on the ${options.boardName} board sits above $${token.symbol}; there is no moon to draw`)
    }
    const headline = rung(token, target)
    const climb = buildLadder(token, eligible, target, options.rungs, options.minMultiple)
    log('target picked', { target: target.symbol, boardRank: target.boardRank, multiple: headline.multiple, excluded: [...exclude], struck: [...struck] })

    // 4. The picture. Logos are a nicety; a missing one is a disc with a letter.
    const [tokenLogo, targetLogo] = await Promise.all([this.logo(token), this.logo(target)])
    const svg = renderMoonSvg({
      token,
      target: headline,
      ladder: climb,
      boardName: options.boardName,
      boardSize: board.length,
      date: today,
      ...(tokenLogo ? { tokenLogo } : {}),
      ...(targetLogo ? { targetLogo } : {}),
      ...(options.watermark ? { watermark: options.watermark } : {}),
    })
    const png = svgToPng(svg)
    let path: string | undefined
    if (options.outputDir) {
      path = join(options.outputDir, `${token.symbol}-vs-${target.symbol.replace(/[^A-Za-z0-9]/g, '')}-${now}.png`)
      mkdirSync(dirname(path), { recursive: true })
      writeFileSync(path, png)
    }
    log('picture drawn', { bytes: png.byteLength, path })

    // 5. The caption, then the filter, the way the morning post reads it.
    let caption = await this.caption({ token, target: headline, ladder: climb, boardSize: board.length, history, community, today })
    let held: string | undefined
    const outgoing = await options.safety?.outbound(caption, { surface: 'moon', kind: 'draft', tool: 'moon_post' })
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

    const base: Omit<MoonResult, 'outcome' | 'postedId'> = {
      token,
      board,
      target: headline,
      ladder: climb,
      excluded: [...exclude],
      svg,
      png,
      ...(path ? { path } : {}),
      caption,
      problems,
      recalled: { history, community },
    }

    // 6. Post, or say what would have been posted.
    if (held) return { ...base, caption: '', outcome: 'held' }
    if (options.dryRun || options.allowPosting === false) {
      await this.capture(now, { ...base, outcome: 'dry-run' })
      return { ...base, outcome: 'dry-run' }
    }
    const uploaded = await options.x.uploadMedia({ data: png, mimeType: 'image/png' })
    const posted = await options.x.createPost(caption, { mediaIds: [uploaded.mediaId] })
    log('posted', { id: posted.id, target: target.symbol })
    const result: MoonResult = { ...base, outcome: 'posted', postedId: posted.id }
    await this.capture(now, result)
    return result
  }

  private async caption(input: {
    token: ListedCoin
    target: Rung
    ladder: Rung[]
    boardSize: number
    history: string
    community: string
    today: string
  }): Promise<string> {
    const { maxEmoji } = this.options
    const context = { persona: this.options.persona, botUsername: this.options.botUsername, today: input.today }
    const ask = moonCaptionPrompt(context, {
      ...input,
      boardName: this.options.boardName,
      maxEmoji,
      maxLength: MAX_POST_LENGTH,
    })
    let { text } = await askJson(this.options.text, { ...ask, schema: moonCaptionSchema })
    if (text.length > MAX_POST_LENGTH || countEmoji(text) > maxEmoji) {
      const retry = await askJson(this.options.text, {
        system: ask.system,
        prompt:
          `${ask.prompt}\n\nYour draft was ${text.length} characters with ${countEmoji(text)} ` +
          `emoji:\n${text}\n\nRewrite it under ${MAX_POST_LENGTH} characters with at most ${maxEmoji} emoji.`,
        schema: moonCaptionSchema,
      })
      text = retry.text
    }
    return applyHouseStyle(text, { maxEmoji, maxLength: MAX_POST_LENGTH }).text
  }

  private async logo(coin: ListedCoin): Promise<Logo | undefined> {
    if (!coin.imageUrl) return undefined
    return this.options.source.logo(coin.imageUrl).catch(() => undefined)
  }

  private async recall(query: string): Promise<string> {
    return this.options.memory.recall(query).catch(() => '')
  }

  /**
   * What went out, with the yardstick's ticker in it, because that is what
   * tomorrow's run reads the exclusion list from. Dry runs are captured
   * too, labelled, like the chart's and the morning's.
   */
  private async capture(now: number, result: MoonResult): Promise<void> {
    const id = result.postedId ?? `dry-run:${now}`
    await this.options.memory
      .captureActivity('moon_post', id, {
        text: result.caption,
        token: result.token.symbol,
        yardstick: result.target.coin.symbol,
        ...(result.token.boardRank === undefined ? {} : { boardRank: result.token.boardRank }),
        yardstickName: result.target.coin.name,
        multiple: Math.round(result.target.multiple * 100) / 100,
        impliedPriceUsd: result.target.impliedPriceUsd,
        marketCapUsd: result.token.marketCapUsd,
        published: result.outcome === 'posted',
      })
      .catch((error: unknown) => {
        result.problems.push(`could not remember the post: ${error instanceof Error ? error.message : String(error)}`)
      })
  }
}

/**
 * Whether a ticker can be drawn and read. The board carries a coin whose
 * symbol is a single ideographic space, one that is `q*`, and a few in
 * Chinese; the font on the picture has no glyphs for some and a caption
 * cannot say the others. They keep their place on the board and are never
 * the comparison.
 */
export function printable(symbol: string): boolean {
  return /^[A-Za-z0-9][A-Za-z0-9._-]{0,15}$/.test(symbol)
}
