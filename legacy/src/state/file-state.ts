import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'
import type { BotState, StateStore } from '../types.js'

const EMPTY_STATE: BotState = { repliedPostIds: [] }

export class FileStateStore implements StateStore {
  constructor(private readonly path: string) {}

  async load(): Promise<BotState> {
    try {
      const contents = await readFile(this.path, 'utf8')
      const parsed = JSON.parse(contents) as Partial<BotState>
      return {
        ...(parsed.sinceId ? { sinceId: parsed.sinceId } : {}),
        ...(parsed.lastPolledAt ? { lastPolledAt: parsed.lastPolledAt } : {}),
        repliedPostIds: parsed.repliedPostIds ?? [],
      }
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return EMPTY_STATE
      throw error
    }
  }

  async save(state: BotState): Promise<void> {
    await mkdir(dirname(this.path), { recursive: true })
    const temporaryPath = `${this.path}.tmp`
    await writeFile(temporaryPath, `${JSON.stringify(state, null, 2)}\n`)
    await rename(temporaryPath, this.path)
  }
}
