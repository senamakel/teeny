import 'dotenv/config'
import { V1Client } from 'cortexdbai/v1'
import { loadConfig } from '../config.js'
import { CortexMemory } from '../memory/cortex.js'
import type { XPost } from '../types.js'

/**
 * Rewrites memory captured before conversations were attributed to people.
 *
 *   npm run migrate-memory            # report what would change
 *   npm run migrate-memory -- --write # re-capture, then forget the originals
 *
 * Early events were stored as opaque JSON under the bot's own id, so recall
 * could not tell two people apart. Each one holds the post it came from, which
 * carries the author, so the person can be recovered and the event rewritten as
 * a chat turn attributed to them.
 *
 * Unpublished drafts are not history and are left behind: a dry-run reply was
 * never said to anyone, and importing one teaches the account it said something
 * it did not.
 */

const config = loadConfig()
const write = process.argv.includes('--write')

const client = new V1Client({
  apiUrl: config.CORTEX_API_URL,
  actor: config.CORTEX_ACTOR,
  bearer: config.CORTEX_TOKEN,
})
const memory = new CortexMemory(
  { apiUrl: config.CORTEX_API_URL, actor: config.CORTEX_ACTOR, bearer: config.CORTEX_TOKEN },
  config.CORTEX_SCOPE,
)

interface StoredEvent {
  id?: string
  observed_actor?: string
  content?: { kind?: string; data?: Record<string, unknown> }
}

const events: StoredEvent[] = []
let cursor: string | undefined
do {
  const page = (await client.events(config.CORTEX_SCOPE, {
    limit: 200,
    ...(cursor ? { cursor } : {}),
  })) as { items?: StoredEvent[]; next_cursor?: string }
  events.push(...(page.items ?? []))
  cursor = page.next_cursor
} while (cursor)

const migrated: string[] = []
const skipped: Array<{ id: string; why: string }> = []

for (const event of events) {
  const { id, content } = event
  if (!id) continue
  if (content?.kind !== 'json') continue

  const data = content.data ?? {}
  const kind = data.kind as string | undefined
  const post = (kind === 'mention' ? data.payload : data.input) as XPost | undefined

  if (kind !== 'mention' && kind !== 'reply') {
    skipped.push({ id, why: `not a conversation (${kind ?? 'unknown'})` })
    continue
  }
  if (!post?.id || !post.text) {
    skipped.push({ id, why: 'no usable post inside' })
    continue
  }
  if (!post.authorId) {
    skipped.push({ id, why: 'no author, so nobody to attribute it to' })
    continue
  }
  if (kind === 'reply' && !data.postedId) {
    skipped.push({ id, why: 'unpublished draft, never said to anyone' })
    continue
  }

  if (write) {
    if (kind === 'mention') {
      await memory.captureMention(post)
    } else {
      await memory.captureReply(post, String(data.reply ?? ''), String(data.postedId))
    }
  }
  migrated.push(id)
}

console.log(`${events.length} events read`)
console.log(`${migrated.length} to rewrite as attributed turns`)
for (const { id, why } of skipped) console.log(`  skipped ${id}: ${why}`)

if (!write) {
  console.log('\nReport only. Re-run with --write to rewrite and remove the originals.')
  process.exit(0)
}

// Only the originals that were successfully rewritten are removed, so nothing
// is deleted that was not first replaced.
const preview = await client.forgetPreview(config.CORTEX_SCOPE, {
  selector: { memory_ids: migrated },
  cascade: 'redact_events',
})
console.log('\nforget preview:', JSON.stringify(preview.estimated_deleted))

const result = await client.forget(config.CORTEX_SCOPE, {
  selector: { memory_ids: migrated },
  cascade: 'redact_events',
  // Deliberately no confirmAll: with a selector it authorizes a SCOPE-WIDE
  // forget and the selector is ignored. The server rejects the combination,
  // which is the only reason this was caught rather than emptying the scope.
  reason: 'superseded by the same conversation attributed to its author',
  ...(preview.preview_id ? { fromPreviewId: preview.preview_id } : {}),
})
console.log('forgot originals:', JSON.stringify(result))
