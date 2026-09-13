import { describe, expect, it } from 'vitest'
import { MongoClient } from 'mongodb'
import {
  MemoryChatTranscript,
  MongoChatTranscript,
  estimateTokens,
  fitToBudget,
} from '../src/telegram/transcript.js'

const line = (n: number, from: 'person' | 'bot' = 'person') => ({
  from,
  ...(from === 'person' ? { authorName: 'Alice' } : {}),
  text: `line ${n} of the conversation`,
})

describe('fitToBudget', () => {
  it('keeps the newest lines that fit and returns them oldest first', () => {
    const newestFirst = [line(5), line(4), line(3), line(2), line(1)]
    const perLine = estimateTokens(line(1).text) + 6
    const kept = fitToBudget(newestFirst, perLine * 2 + 1)
    expect(kept.map((turn) => turn.text)).toEqual([line(4).text, line(5).text])
  })

  it('keeps a single oversized newest line rather than nothing', () => {
    const huge = { from: 'person' as const, text: 'x'.repeat(10_000) }
    expect(fitToBudget([huge, line(1)], 50)).toEqual([huge])
  })

  it('keeps everything when the budget allows', () => {
    expect(fitToBudget([line(2), line(1)], 5_000)).toHaveLength(2)
  })
})

describe('MemoryChatTranscript', () => {
  it('stores per chat, drops redeliveries, and excludes the message being answered', async () => {
    const store = new MemoryChatTranscript()
    const at = '2026-09-14T00:00:00.000Z'
    await store.append({ chatId: '1', messageId: 'a', at, from: 'person', authorName: 'A', text: 'hi' })
    await store.append({ chatId: '1', messageId: 'a', at, from: 'person', authorName: 'A', text: 'hi' })
    await store.append({ chatId: '1', messageId: 'b', at, from: 'bot', text: 'hello' })
    await store.append({ chatId: '1', messageId: 'c', at, from: 'person', authorName: 'A', text: 'how are you' })
    await store.append({ chatId: '2', messageId: 'z', at, from: 'person', authorName: 'B', text: 'other room' })

    await expect(store.recent('1', { maxTokens: 5_000, before: 'c' })).resolves.toEqual([
      { from: 'person', authorName: 'A', text: 'hi' },
      { from: 'bot', text: 'hello' },
    ])
    await expect(store.recent('2', { maxTokens: 5_000 })).resolves.toEqual([
      { from: 'person', authorName: 'B', text: 'other room' },
    ])
  })
})

/**
 * Runs against a real Mongo when one is reachable on the default local port,
 * and is skipped otherwise, so the suite does not depend on the box.
 */
const uri = process.env.TEST_MONGO_URI ?? 'mongodb://127.0.0.1:27017'
const reachable = await new MongoClient(uri, { serverSelectionTimeoutMS: 800 })
  .connect()
  .then((client) => client.close().then(() => true))
  .catch(() => false)

describe.skipIf(!reachable)('MongoChatTranscript', () => {
  it('round-trips turns oldest first within the budget and survives redelivery', async () => {
    const collection = `transcript_test_${Date.now()}`
    const store = new MongoChatTranscript({ uri, database: 'teeny_test', collection })
    try {
      const base = Date.parse('2026-09-14T00:00:00.000Z')
      for (let i = 1; i <= 5; i += 1) {
        await store.append({
          chatId: 'c',
          messageId: String(i),
          at: new Date(base + i * 1_000).toISOString(),
          from: i % 2 ? 'person' : 'bot',
          ...(i % 2 ? { authorName: 'Alice' } : {}),
          text: `line ${i}`,
        })
      }
      await store.append({ chatId: 'c', messageId: '3', at: 'later', from: 'person', text: 'dup' })

      const all = await store.recent('c', { maxTokens: 5_000, before: '5' })
      expect(all.map((turn) => turn.text)).toEqual(['line 1', 'line 2', 'line 3', 'line 4'])

      const some = await store.recent('c', { maxTokens: 20 })
      expect(some.map((turn) => turn.text)).toEqual(['line 4', 'line 5'])
    } finally {
      await new MongoClient(uri).db('teeny_test').collection(collection).drop().catch(() => undefined)
      await store.close()
    }
  })
})
