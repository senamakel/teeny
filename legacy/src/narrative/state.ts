import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'

/**
 * Where the serial is. Memory remembers the episodes as text, and that is
 * what the writers read for texture; this file is the ledger: which episode
 * comes next, which lore has come out, what was left hanging. Recall is
 * fuzzy and dry runs pollute it, and a serial that loses count of itself is
 * a serial that reveals the same secret twice.
 */
export interface NarrativeState {
  season: number
  /** The number of the last episode published. The next one is this plus one. */
  episode: number
  surfacedLore: string[]
  openThreads: string[]
  lastEpisodeId?: string
  lastTitle?: string
  lastLogline?: string
  stateAfter?: string
}

export const FIRST_STATE: NarrativeState = { season: 1, episode: 0, surfacedLore: [], openThreads: [] }

export interface NarrativeStateStore {
  load(): Promise<NarrativeState>
  save(state: NarrativeState): Promise<void>
}

export class FileNarrativeState implements NarrativeStateStore {
  constructor(private readonly path: string) {}

  async load(): Promise<NarrativeState> {
    try {
      const parsed = JSON.parse(await readFile(this.path, 'utf8')) as Partial<NarrativeState>
      return {
        season: parsed.season ?? 1,
        episode: parsed.episode ?? 0,
        surfacedLore: parsed.surfacedLore ?? [],
        openThreads: parsed.openThreads ?? [],
        ...(parsed.lastEpisodeId ? { lastEpisodeId: parsed.lastEpisodeId } : {}),
        ...(parsed.lastTitle ? { lastTitle: parsed.lastTitle } : {}),
        ...(parsed.lastLogline ? { lastLogline: parsed.lastLogline } : {}),
        ...(parsed.stateAfter ? { stateAfter: parsed.stateAfter } : {}),
      }
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return { ...FIRST_STATE }
      throw error
    }
  }

  async save(state: NarrativeState): Promise<void> {
    await mkdir(dirname(this.path), { recursive: true })
    const temporaryPath = `${this.path}.tmp`
    await writeFile(temporaryPath, `${JSON.stringify(state, null, 2)}\n`)
    await rename(temporaryPath, this.path)
  }
}

export class MemoryNarrativeState implements NarrativeStateStore {
  constructor(private state: NarrativeState = { ...FIRST_STATE }) {}
  async load(): Promise<NarrativeState> {
    return { ...this.state, surfacedLore: [...this.state.surfacedLore], openThreads: [...this.state.openThreads] }
  }
  async save(state: NarrativeState): Promise<void> {
    this.state = state
  }
}
