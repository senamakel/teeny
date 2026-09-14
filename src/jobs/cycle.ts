/**
 * A rota for the posting jobs.
 *
 * The moon chart, the candlestick chart and a quiet day take turns: day one
 * the moon, day two the chart, day three nothing. Cron cannot say "every
 * third day" -- a step of three in the day-of-month field restarts at every month and
 * lands two posts back to back on the 31st and the 1st -- so every job keeps
 * its daily cron and asks the rota whether today is its turn. Days are
 * counted in UTC from a fixed anchor, so the answer is the same in every
 * process and after every restart.
 */
export interface Rota {
  /** How many days one turn of the rota takes. */
  days: number
  /** ISO date the count starts from; that day is slot 0. */
  anchor: string
}

const DAY_MS = 24 * 60 * 60 * 1_000

/** Which slot of the rota a moment falls in, 0-based. */
export function rotaSlot(rota: Rota, now: number): number {
  const start = Date.parse(`${rota.anchor}T00:00:00Z`)
  if (!Number.isFinite(start)) throw new Error(`rota anchor "${rota.anchor}" is not an ISO date`)
  const elapsed = Math.floor((now - start) / DAY_MS)
  // A negative modulo would put the days before the anchor on the wrong slot.
  return ((elapsed % rota.days) + rota.days) % rota.days
}

/** Whether a job with this slot is on today. A slot past the rota's length is never on. */
export function isTurn(rota: Rota, slot: number, now: number): boolean {
  return rotaSlot(rota, now) === slot
}
