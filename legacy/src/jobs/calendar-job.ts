import { applyHouseStyle } from '../agent/house-style.js'
import type { ContentDraft } from '../agent/content-model.js'
import type { CalendarItem } from '../calendar/calendar.js'
import type { TelegramApi } from '../clients/telegram.js'
import { TELEGRAM_MAX_CAPTION_LENGTH } from '../clients/telegram.js'
import type { Journal, Outcome } from '../journal/journal.js'
import type { Cadence } from '../cadence/cadence.js'
import type { MediaStore } from '../media/store.js'
import type { SafetyGate } from '../safety/gate.js'
import type { Memory, XTools } from '../types.js'

export interface ContentDrafter {
  draft(item: CalendarItem, input: { today: string; memory: string }): Promise<ContentDraft>
}

export interface CalendarJobDependencies {
  agent: ContentDrafter
  memory: Pick<Memory, 'recall' | 'captureActivity'>
  x: Pick<XTools, 'createPost' | 'uploadMedia'>
  /** Absent: items with a telegram channel log a warning and post to X only. */
  telegram?: Pick<TelegramApi, 'sendMessage' | 'sendPhoto' | 'sendVideo'>
  /** Where the drafter's generated file is; absent when nothing can generate. */
  media?: MediaStore
  logger: {
    info: (object: unknown, message?: string) => void
    warn: (object: unknown, message?: string) => void
    error: (object: unknown, message?: string) => void
  }
  dryRun: boolean
  /** Chats an item posts to when it names none of its own. */
  telegramChatIds?: string[]
  maxEmoji?: number
  /** Refuses to run the same item again inside this window. Defaults to an hour. */
  minIntervalMs?: number
  now?: () => number
  journal?: Journal
  /**
   * The content filter, outbound. A calendar post is the account's own words
   * with nobody to answer, so only the out gate applies: a draft it holds is
   * not published on any channel, and a link it cuts stays cut. The picture
   * is gated separately, in the generate tool the drafter was given.
   */
  safety?: Pick<SafetyGate, 'outbound'>
  /**
   * The content calendar's spacing rule. Asked before anything is drawn or
   * written, so a slot that is too close to the last post, or held for a
   * hot one, costs no generation. Only X is subject to it; a Telegram-only item posts regardless.
   */
  cadence?: Pick<Cadence, 'claim' | 'within'>
}

/**
 * The content calendar, run: one item at a time, on its cron.
 *
 * This is where the account generates media without being asked (the morning
 * post, a fixed pipeline rather than a drafter, is the other): the item says
 * whether it carries a picture, and the drafter has the tool for exactly
 * that. The bargain is kept the other way too -- an item that promised a
 * picture and did not get one is held, not published bare, since a
 * "gm" with the image missing reads as a broken bot to everybody who has seen
 * the ones with it.
 *
 * The per-item interval guard is what the chart job has, for the same reason:
 * a container that restarts near the cron minute fires again, and the same
 * post twice in an hour is the pattern that once cost this account its reach.
 */
export class CalendarJob {
  private readonly lastRunAt = new Map<string, number>()

  constructor(private readonly dependencies: CalendarJobDependencies) {}

  /** Returns false when it declined to run, which is not an error. */
  async run(item: CalendarItem, options: { force?: boolean } = {}): Promise<boolean> {
    const { logger } = this.dependencies
    const now = (this.dependencies.now ?? Date.now)()
    const minimum = this.dependencies.minIntervalMs ?? 60 * 60_000
    const last = this.lastRunAt.get(item.name) ?? 0
    if (!options.force && last > 0 && now - last < minimum) {
      logger.info({ item: item.name, sinceLastRunMs: now - last }, 'calendar: skipping, it just ran')
      return false
    }
    if (item.channels.includes('x') && this.dependencies.cadence) {
      const claim = await this.dependencies.cadence.claim(`calendar:${item.name}`)
      if (!claim.ok && !options.force) {
        logger.info({ item: item.name, ...claim }, `calendar: skipping, the cadence says ${claim.reason === 'gap' ? 'too soon after the last post' : 'the timeline is held for a hot post'}`)
        return false
      }
    }
    this.lastRunAt.set(item.name, now)

    const today = new Date(now).toISOString().slice(0, 10)
    const id = `calendar:${item.name}:${today}`
    // Recall fails soft: a post that cannot see yesterday's is a worse post,
    // a day with no post because memory was down is a missing day.
    const memory = await Promise.resolve(
      this.dependencies.memory.recall(
        `What has this account posted for its scheduled "${item.name}" calendar item on earlier ` +
          'days? For each: the text, and the scene of the picture if it had one.',
      ),
    ).catch(() => '')

    let draft: ContentDraft
    try {
      draft = await this.dependencies.agent.draft(item, { today, memory })
    } catch (error) {
      logger.error({ err: error, item: item.name }, 'calendar: drafting failed')
      await this.journal(id, now, item, undefined, 'failed', error instanceof Error ? error.message : String(error))
      return true
    }

    const styled = applyHouseStyle(draft.text, {
      maxEmoji: this.dependencies.maxEmoji ?? 1,
      maxLength: 280,
    })
    if (styled.fixes.length > 0) logger.info({ item: item.name, fixes: styled.fixes }, 'calendar: house style applied')
    let text = styled.text

    const generated = draft.mediaId ? this.dependencies.media?.get(draft.mediaId) : undefined
    if (item.media !== 'none' && !generated) {
      const note = draft.mediaId
        ? `the drafted mediaId ${draft.mediaId} is not in the store`
        : `the item carries ${item.media === 'image' ? 'an image' : 'a video'} and none was generated`
      logger.warn({ item: item.name, note }, 'calendar: holding the post rather than publishing it bare')
      await this.journal(id, now, item, { ...draft, text }, 'held', note)
      return true
    }
    if (!text && !generated) {
      await this.journal(id, now, item, { ...draft, text }, 'held', 'the draft was empty')
      return true
    }

    // The gate, outbound: the styled text is what would go out on every
    // channel, so that is what is checked, and what comes back (links cut)
    // is what goes out. Checked before the dry run, so a dry run shows the
    // filter's verdict too. A held post is a missing day, which is the right
    // outcome for a post the account should not have written; the item runs
    // again on its next cron.
    const outgoing = await this.dependencies.safety?.outbound(text, {
      surface: 'calendar',
      kind: 'draft',
      tool: item.name,
    })
    if (outgoing) {
      if (outgoing.fixes.length > 0) {
        logger.warn({ item: item.name, fixes: outgoing.fixes }, 'calendar: the content filter edited the draft')
      }
      if (!outgoing.decision.ok) {
        logger.warn(
          {
            item: item.name,
            category: outgoing.decision.category,
            by: outgoing.decision.by,
            reason: outgoing.decision.reason,
          },
          'calendar: withheld the post, the content filter refused the draft',
        )
        await this.journal(
          id,
          now,
          item,
          { ...draft, text: '' },
          'held',
          `content filter withheld the draft: ${outgoing.decision.category}`,
        )
        return true
      }
      text = outgoing.text
    }

    if (this.dependencies.dryRun) {
      logger.info(
        { item: item.name, channels: item.channels, text, mediaId: generated?.id, kind: generated?.kind },
        'DRY RUN: would have published a calendar post',
      )
      await this.remember(id, item, text, generated?.id, false)
      await this.journal(id, now, item, { ...draft, text }, 'dry-run')
      return true
    }

    const results: string[] = []
    const failures: string[] = []
    if (item.channels.includes('x')) {
      try {
        const mediaIds = generated
          ? [(await this.dependencies.x.uploadMedia({ data: generated.data, mimeType: generated.mimeType })).mediaId]
          : undefined
        const publish = () => this.dependencies.x.createPost(text || ' ', mediaIds ? { mediaIds } : {})
        const posted = this.dependencies.cadence
          ? await this.dependencies.cadence.within(`calendar:${item.name}`, publish)
          : await publish()
        results.push(`x:${posted.id}`)
      } catch (error) {
        failures.push(`x: ${error instanceof Error ? error.message : String(error)}`)
        logger.error({ err: error, item: item.name }, 'calendar: could not post to X')
      }
    }
    if (item.channels.includes('telegram')) {
      const chats = item.telegramChatIds ?? this.dependencies.telegramChatIds ?? []
      const telegram = this.dependencies.telegram
      if (!telegram || chats.length === 0) {
        failures.push('telegram: no client or no chat ids configured')
        logger.warn({ item: item.name }, 'calendar: telegram channel named but nowhere to post')
      }
      for (const chat of chats) {
        if (!telegram) break
        try {
          const chatId = Number(chat)
          const caption = text.length <= TELEGRAM_MAX_CAPTION_LENGTH ? text : undefined
          let sent: { id: number }
          if (generated) {
            const upload = { data: generated.data, mimeType: generated.mimeType, ...(caption ? { caption } : {}) }
            sent =
              generated.kind === 'video'
                ? await telegram.sendVideo(chatId, upload)
                : await telegram.sendPhoto(chatId, upload)
            if (!caption && text) await telegram.sendMessage(chatId, text)
          } else {
            sent = await telegram.sendMessage(chatId, text)
          }
          results.push(`telegram:${chat}:${sent.id}`)
        } catch (error) {
          failures.push(`telegram ${chat}: ${error instanceof Error ? error.message : String(error)}`)
          logger.error({ err: error, item: item.name, chat }, 'calendar: could not post to a telegram chat')
        }
      }
    }

    const outcome: Outcome = results.length > 0 ? 'posted' : 'failed'
    logger.info({ item: item.name, results, failures, text, mediaId: generated?.id }, 'calendar: done')
    if (results.length > 0) await this.remember(id, item, text, generated?.id, true, results)
    await this.journal(id, now, item, { ...draft, text }, outcome, failures.length > 0 ? failures.join('; ') : undefined)
    return true
  }

  /** Written back deterministically, so tomorrow's run can see today's scene. */
  private async remember(
    id: string,
    item: CalendarItem,
    text: string,
    mediaId: string | undefined,
    published: boolean,
    results: string[] = [],
  ): Promise<void> {
    await Promise.resolve(
      this.dependencies.memory.captureActivity('calendar_post', id, {
        item: item.name,
        text,
        channels: item.channels,
        ...(mediaId ? { media: item.media, mediaId, prompt: this.dependencies.media?.get(mediaId)?.prompt } : {}),
        published,
        results,
      }),
    ).catch((error: unknown) =>
      this.dependencies.logger.warn({ err: error, item: item.name }, 'calendar: could not remember the post'),
    )
  }

  private async journal(
    id: string,
    now: number,
    item: CalendarItem,
    draft: ContentDraft | undefined,
    outcome: Outcome,
    note?: string,
  ): Promise<void> {
    const { journal, logger } = this.dependencies
    if (!journal) return
    try {
      await journal.record({
        id,
        at: new Date(now).toISOString(),
        trigger: 'calendar',
        incoming: `${item.name}: ${item.brief}`,
        tone: 'neutral',
        recalled: [],
        tools: draft?.tools ?? [],
        thinking: draft?.thinking ?? '',
        said: outcome === 'posted' || outcome === 'dry-run' ? (draft?.text ?? '') : '',
        declined: draft?.declined ?? [],
        outcome,
        ...(note ? { outcomeNote: note } : {}),
      })
    } catch (error) {
      logger.warn({ err: error, item: item.name }, 'calendar: could not journal the turn')
    }
  }
}
