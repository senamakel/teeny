import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { FileStateStore } from '../src/state/file-state.js'

describe('FileStateStore', () => {
  it('round-trips every field, so the poll window survives a restart', async () => {
    const store = new FileStateStore(join(mkdtempSync(join(tmpdir(), 'state-')), 'state.json'))
    const state = {
      sinceId: '42',
      lastPolledAt: '2026-09-01T22:00:00.000Z',
      repliedPostIds: ['1', '2'],
    }

    await store.save(state)
    // A field saved but not loaded reads as absent, and the job silently falls
    // back to its lookback default on every run instead of the last poll.
    await expect(store.load()).resolves.toEqual(state)
  })

  it('returns empty state when the file does not exist', async () => {
    const store = new FileStateStore(join(tmpdir(), 'definitely-missing', 'state.json'))
    await expect(store.load()).resolves.toEqual({ repliedPostIds: [] })
  })
})
