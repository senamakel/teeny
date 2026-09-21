import { rotaSlot, type Rota } from './cycle.js'

export interface SlotOwner {
  name: string
  /** The rota slots (days) this job owns in the slot. */
  slots: number[]
  run(): Promise<unknown>
}

export interface SlotJobDependencies {
  /** Which slot of the day this is, for the log. */
  label: string
  rota: Rota
  owners: SlotOwner[]
  /** Runs on every day no owner has. Absent, those days are quiet. */
  fallback?: { name: string; run(): Promise<unknown> }
  logger: { info: (object: unknown, message?: string) => void; error: (object: unknown, message?: string) => void }
  now?: () => number
}

/**
 * One slot of the day's calendar, on one cron: which job posts in it is
 * the rota's to say.
 *
 * The afternoon slot used to be three crons at the same minute, each
 * asking the rota whether today was its turn; a fourth kind of post at
 * the same hour would have been a fourth cron and a fourth check. This is
 * the same rule in one place: the slot fires, the day's slot number picks
 * the owner, the owner runs, and the fallback takes every day nobody
 * claimed -- so switching a job off hands its days to the fallback rather
 * than leaving them quiet. Each owner keeps its own guards (its interval,
 * the cadence), so a restart near the cron minute is still safe.
 */
export class SlotJob {
  constructor(private readonly dependencies: SlotJobDependencies) {}

  /** Who owns each slot of the rota, for the startup log: `0:moon 1:lore ...`. */
  describe(): string {
    return Array.from({ length: this.dependencies.rota.days }, (_, slot) => `${slot}:${this.ownerOf(slot)?.name ?? '-'}`).join(' ')
  }

  private ownerOf(slot: number): { name: string; run(): Promise<unknown> } | undefined {
    return this.dependencies.owners.find((owner) => owner.slots.includes(slot)) ?? this.dependencies.fallback
  }

  async run(): Promise<void> {
    const { dependencies } = this
    const now = (dependencies.now ?? Date.now)()
    const slot = rotaSlot(dependencies.rota, now)
    const owner = this.ownerOf(slot)
    if (!owner) {
      dependencies.logger.info({ slot: dependencies.label, day: slot }, 'slot: nobody has today; quiet')
      return
    }
    dependencies.logger.info({ slot: dependencies.label, day: slot, job: owner.name }, 'slot: running the day\'s job')
    try {
      await owner.run()
    } catch (error) {
      dependencies.logger.error({ err: error, slot: dependencies.label, job: owner.name }, 'slot: the day\'s job failed')
    }
  }
}
