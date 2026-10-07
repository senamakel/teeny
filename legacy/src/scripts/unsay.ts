import 'dotenv/config'
import { createHmac, randomBytes } from 'node:crypto'
import { MongoClient } from 'mongodb'
import { loadConfig } from '../config.js'

/**
 * Deletes replies the account should not have made, and queues the mentions
 * behind them to be answered again.
 *
 *   npm run unsay -- <reply-id> [<reply-id> ...]
 *   npm run unsay -- <reply-id> --confirm
 *
 * The account said several dismissive things before the voice was fixed --
 * calling a friendly post a shill, and telling people it had no tokens to give
 * on the day it was given a wallet. Deleting those is only half of it: the
 * mention they answered is marked `replied`, so nothing would ever answer it
 * again. This puts it back to `pending`, and the reply job picks it up on the
 * next tick with whatever the prompt says now.
 *
 * A deleted post is gone for good; the mention row is not, so the second half
 * is reversible and the first is not. Nothing happens without --confirm.
 */

const config = loadConfig()
const confirm = process.argv.includes('--confirm')
const ids = process.argv.slice(2).filter((argument) => /^\d+$/.test(argument))

if (ids.length === 0) {
  console.error('usage: npm run unsay -- <reply-id> [<reply-id> ...] [--confirm]')
  process.exit(2)
}

const enc = (value: string): string =>
  encodeURIComponent(value).replace(
    /[!*'()]/g,
    (character) => `%${character.charCodeAt(0).toString(16).toUpperCase()}`,
  )

/** The same OAuth 1.0a signing the webhook script uses; see its header. */
function oauth1Header(method: string, url: string): string {
  const parsed = new URL(url)
  const oauth: Record<string, string> = {
    oauth_consumer_key: config.X_API_KEY,
    oauth_nonce: randomBytes(16).toString('hex'),
    oauth_signature_method: 'HMAC-SHA1',
    oauth_timestamp: Math.floor(Date.now() / 1000).toString(),
    oauth_token: config.X_ACCESS_TOKEN,
    oauth_version: '1.0',
  }
  const parameters = Object.keys(oauth)
    .sort()
    .map((key) => `${enc(key)}=${enc(oauth[key] ?? '')}`)
    .join('&')
  const base = [
    method.toUpperCase(),
    enc(`${parsed.origin}${parsed.pathname}`),
    enc(parameters),
  ].join('&')
  const key = `${enc(config.X_API_SECRET)}&${enc(config.X_ACCESS_TOKEN_SECRET)}`
  const header = { ...oauth, oauth_signature: createHmac('sha1', key).update(base).digest('base64') }
  return `OAuth ${Object.keys(header)
    .sort()
    .map((name) => `${enc(name)}="${enc(header[name as keyof typeof header] ?? '')}"`)
    .join(', ')}`
}

async function repliedTo(id: string): Promise<string | undefined> {
  const response = await fetch(
    `https://api.x.com/2/tweets/${id}?tweet.fields=referenced_tweets`,
    { headers: { authorization: `Bearer ${config.X_BEARER_TOKEN ?? ''}` } },
  )
  const body = (await response.json()) as {
    data?: { referenced_tweets?: Array<{ id: string; type: string }> }
  }
  return body.data?.referenced_tweets?.find((entry) => entry.type === 'replied_to')?.id
}

async function main(): Promise<void> {
  const mongo = new MongoClient(config.MONGO_URI)
  await mongo.connect()
  const mentions = mongo.db(config.MONGO_DATABASE).collection(config.MENTIONS_COLLECTION)

  for (const id of ids) {
    const answered = await repliedTo(id)
    const mention = answered ? await mentions.findOne({ postId: answered }) : null

    if (!confirm) {
      console.log(
        `would delete ${id}` +
          (mention ? ` and requeue ${answered} (${String(mention.status)})` : ' (no mention found)'),
      )
      continue
    }

    const url = `https://api.x.com/2/tweets/${id}`
    const response = await fetch(url, {
      method: 'DELETE',
      headers: { authorization: oauth1Header('DELETE', url) },
    })
    const body = (await response.json()) as { data?: { deleted?: boolean }; detail?: string }
    if (!body.data?.deleted) {
      console.error(`  could not delete ${id}: ${body.detail ?? JSON.stringify(body)}`)
      continue
    }
    console.log(`deleted ${id}`)

    if (answered) {
      // Cleared, not just reset: the attempt count and the old error belong to
      // an answer that no longer exists.
      const result = await mentions.updateOne(
        { postId: answered },
        {
          $set: { status: 'pending', attempts: 0, updatedAt: new Date().toISOString() },
          $unset: { error: '', reply: '' },
        },
      )
      console.log(
        result.matchedCount > 0
          ? `  requeued ${answered} for a fresh reply`
          : `  no mention row for ${answered}; nothing to requeue`,
      )
    }
  }

  await mongo.close()
  if (!confirm) console.log('\nNothing was changed. Re-run with --confirm.')
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : error)
  process.exit(1)
})
