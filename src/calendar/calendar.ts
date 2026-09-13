import { readFileSync } from 'node:fs'
import cron from 'node-cron'
import { z } from 'zod'

/**
 * One scheduled post. `brief` is what the account is asked to write about, in
 * the operator's words; the model turns it into the post in its own voice.
 * `media` is a promise as much as a permission: `image` means the post
 * carries one, generated for it, and the job refuses to publish text alone
 * when the picture did not happen.
 */
const item = z.object({
  /** Stable key: journal ids, memory, and the "already ran today" guard all use it. */
  name: z.string().regex(/^[a-z0-9][a-z0-9-]{0,39}$/, 'lowercase letters, digits and dashes'),
  cron: z.string().refine((value) => cron.validate(value), 'not a cron expression'),
  channels: z.array(z.enum(['x', 'telegram'])).min(1),
  brief: z.string().min(10).max(4_000),
  media: z.enum(['none', 'image', 'video']).default('none'),
  /** For an image: square, landscape, portrait. For a video: square, landscape, portrait. */
  aspect: z.enum(['square', 'landscape', 'portrait']).default('square'),
  /** Whether Tiny is in the picture. Almost always. */
  featuresTiny: z.boolean().default(true),
  /** Telegram chats to post to; falls back to CALENDAR_TELEGRAM_CHAT_IDS. */
  telegramChatIds: z.array(z.string().regex(/^-?\d+$/)).optional(),
  enabled: z.boolean().default(true),
})

const schema = z.object({
  /** An IANA zone every cron below is read in. Defaults to the service's. */
  timezone: z.string().min(1).optional(),
  items: z.array(item).min(1),
})

export type CalendarItem = z.infer<typeof item>
export type Calendar = z.infer<typeof schema>

/**
 * The content calendar: a JSON file of scheduled posts, read at startup.
 *
 * A file rather than configuration because a calendar is a list and .env is
 * not; JSON rather than Markdown because a cron expression and a channel list
 * want a schema, and because the briefs are short. Names must be unique: two
 * items called `gm` would share a journal id and a memory key, and the second
 * would look like a rerun of the first.
 */
export function parseCalendar(text: string): Calendar {
  const parsed = schema.parse(JSON.parse(text))
  const names = new Set<string>()
  for (const entry of parsed.items) {
    if (names.has(entry.name)) throw new Error(`calendar: two items are named "${entry.name}"`)
    names.add(entry.name)
  }
  return parsed
}

export function loadCalendar(path: string): Calendar {
  return parseCalendar(readFileSync(path, 'utf8'))
}
