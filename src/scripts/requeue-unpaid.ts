import 'dotenv/config'
import { MongoClient } from 'mongodb'
import { loadConfig } from '../config.js'
import type { XPost } from '../types.js'

/**
 * Finds people who asked, gave a wallet address, and never got anything, and
 * puts their mention back in the queue to be answered again.
 *
 *   npm run requeue-unpaid            # list them, change nothing
 *   npm run requeue-unpaid -- --confirm
 *
 * The day's budget ran out mid-campaign, so the account spent an hour telling
 * people to come back tomorrow. Their mentions are marked `replied`, which
 * means nothing would ever revisit them: the answer they got was "not today",
 * and without this that is the last word.
 *
 * The test is deliberately not "what did the reply say". It is: their post
 * contains an address, and the ledger has never paid that account. That covers
 * a budget refusal, a model error mid-turn, and a transfer that reverted, and
 * it cannot mistakenly pay somebody twice -- if there is a row for them, they
 * are not in this list.
 *
 * Requeueing does not promise anybody anything. The turn runs again from the
 * top, so a person who still does not follow, or a wallet already claimed by
 * another account, is refused exactly as before.
 */

const config = loadConfig()
const confirm = process.argv.includes('--confirm')
const ADDRESS = /0x[0-9a-fA-F]{40}/

interface MentionRow {
  postId: string
  post: XPost
  status: string
  error?: string
}

async function main(): Promise<void> {
  const mongo = new MongoClient(config.MONGO_URI)
  await mongo.connect()
  const database = mongo.db(config.MONGO_DATABASE)
  const mentions = database.collection<MentionRow>(config.MENTIONS_COLLECTION)
  const airdrops = database.collection(config.AIRDROPS_COLLECTION)

  // Anyone with a row, whatever its state: a `sending` row means a transfer may
  // be in flight, and paying again to be helpful is the one mistake that cannot
  // be taken back.
  const paid = new Set(
    (await airdrops.find({}, { projection: { recipientUserId: 1 } }).toArray())
      .map((row) => (row as { recipientUserId?: string }).recipientUserId)
      .filter((id): id is string => Boolean(id)),
  )

  const answered = await mentions.find({ status: { $in: ['replied', 'failed'] } }).toArray()
  const unpaid = answered.filter(
    (row) => ADDRESS.test(row.post?.text ?? '') && !paid.has(row.post?.authorId ?? ''),
  )

  console.log(`${unpaid.length} asked, gave an address, and were never paid.\n`)
  for (const row of unpaid) {
    console.log(
      `  @${row.post.authorUsername ?? '?'} (${row.status})` +
        `  ${row.post.text.replace(/\s+/g, ' ').slice(0, 70)}`,
    )
  }

  if (!confirm) {
    console.log('\nNothing was changed. Re-run with --confirm to queue them for another answer.')
    await mongo.close()
    return
  }

  const result = await mentions.updateMany(
    { postId: { $in: unpaid.map((row) => row.postId) } },
    {
      $set: { status: 'pending', attempts: 0, updatedAt: new Date().toISOString() },
      $unset: { error: '' },
    },
  )
  console.log(`\nRequeued ${result.modifiedCount}. The reply worker will pick them up.`)
  await mongo.close()
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : error)
  process.exit(1)
})
