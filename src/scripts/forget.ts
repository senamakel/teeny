import 'dotenv/config'
import { V1Client } from 'cortexdbai/v1'
import { loadConfig } from '../config.js'

/**
 * Previews, and only with `--confirm` performs, a CortexDB deletion.
 *
 *   npm run forget -- --find "droplet"          # list matching events and ids
 *   npm run forget -- --ids id1,id2             # preview deleting those
 *   npm run forget -- --ids id1,id2 --confirm   # delete them
 *   npm run forget -- "<subject>" [--confirm]   # by subject instead
 *
 * Operational notes written during setup are recalled like anything else, and
 * a persona repeats them in public voice, so they have to be removed rather
 * than only discouraged in the prompt.
 */

const config = loadConfig()
const argv = process.argv.slice(2)
const confirm = argv.includes('--confirm')

function flag(name: string): string | undefined {
  const index = argv.indexOf(name)
  return index >= 0 ? argv[index + 1] : undefined
}

const find = flag('--find')
const ids = flag('--ids')?.split(',').map((id) => id.trim()).filter(Boolean)
const subject = argv[0]?.startsWith('--') ? undefined : argv[0]

const client = new V1Client({
  apiUrl: config.CORTEX_API_URL,
  actor: config.CORTEX_ACTOR,
  bearer: config.CORTEX_TOKEN,
})

// Listing first, because a selector that matches nothing looks exactly like a
// selector that matched and found nothing to delete.
if (find) {
  const response = (await client.events(config.CORTEX_SCOPE, { limit: 500 })) as {
    items?: Array<{ id?: string }>
  }
  const needle = find.toLowerCase()
  for (const event of response.items ?? []) {
    const blob = JSON.stringify(event)
    if (blob.toLowerCase().includes(needle)) console.log(`${event.id}\t${blob.slice(0, 400)}`)
  }
  process.exit(0)
}

if (!ids && !subject) {
  console.error('usage: npm run forget -- [--find <text> | --ids <id,id> | "<subject>"] [--confirm]')
  process.exit(2)
}

const selector = ids ? { memory_ids: ids } : { about_subject: subject! }

const preview = await client.forgetPreview(config.CORTEX_SCOPE, {
  selector,
  cascade: 'redact_events',
})
console.log(JSON.stringify(preview, null, 2))

if (!confirm) {
  console.log('\nPreview only. Re-run with --confirm to delete.')
  process.exit(0)
}

const result = await client.forget(config.CORTEX_SCOPE, {
  selector,
  cascade: 'redact_events',
  // Deliberately no confirmAll: with a selector it authorizes a SCOPE-WIDE
  // forget and the selector is ignored. The server rejects the combination,
  // which is the only reason this was caught rather than emptying the scope.
  reason: 'operational note leaking into public replies',
  ...(preview.preview_id ? { fromPreviewId: preview.preview_id } : {}),
})
console.log(JSON.stringify(result, null, 2))
