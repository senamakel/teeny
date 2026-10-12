/**
 * What an operator's decision on a hold actually does.
 *
 * Rejecting confirms the filter and closes the entry. Approving overrules it,
 * and an overrule that changes nothing is a note, not a decision, so each
 * kind of hold has an effect: a cut link puts its host on the live allowlist,
 * a held mention goes back to the queue to be answered, a withheld draft is
 * released as it was written. What cannot be acted on after the fact -- a
 * Telegram message from an hour ago, a picture the account declined to draw
 * -- is recorded as reviewed and nothing more, and the effect says so.
 */
import type { Hold, HoldLog } from './holds.js'

export interface ReviewDependencies {
  holds: HoldLog
  /** Puts an X mention back to be answered with the inbound check waived. */
  requeue?: (postId: string) => Promise<boolean>
  /** Publishes a withheld draft as written. Returns a line on what happened. */
  release?: (hold: Hold) => Promise<string>
  /** Tells a gate in this process to re-read the approved links. */
  refreshLinks?: () => void
}

/** A cut link as an allowlist entry: its host, with subdomains. */
export function allowlistEntryFor(link: string): string | undefined {
  try {
    const url = new URL(/^https?:\/\//i.test(link) ? link : `https://${link}`)
    return url.hostname.toLowerCase() || undefined
  } catch {
    return undefined
  }
}

export async function reviewHold(
  id: string,
  decision: 'approved' | 'rejected',
  deps: ReviewDependencies,
  note?: string,
): Promise<Hold | undefined> {
  const hold = await deps.holds.get(id)
  if (!hold) return undefined
  // A decision is taken once. Re-deciding would re-run the effect: a second
  // publish of a released draft, a second requeue of an answered mention.
  if (hold.status !== 'pending') return hold

  const effect = decision === 'rejected' ? 'confirmed the filter' : await approve(hold, deps)
  return deps.holds.decide(id, decision, { effect, ...(note ? { note } : {}) })
}

async function approve(hold: Hold, deps: ReviewDependencies): Promise<string> {
  switch (hold.kind) {
    case 'link': {
      const hosts = [...new Set((hold.links ?? []).map((entry) => allowlistEntryFor(entry.link)).filter(Boolean))] as string[]
      if (hosts.length === 0) return 'no usable host in the cut links; nothing allowlisted'
      for (const host of hosts) await deps.holds.allowLink(host, { holdId: hold.id })
      deps.refreshLinks?.()
      return `allowlisted ${hosts.join(', ')}`
    }
    case 'message': {
      if (hold.context.surface !== 'x' || !hold.context.postId) {
        return 'recorded; a held chat message cannot be answered after the fact'
      }
      if (!deps.requeue) return 'recorded; no queue to return it to'
      return (await deps.requeue(hold.context.postId))
        ? 'requeued to be answered'
        : 'recorded; the mention is no longer in the queue'
    }
    case 'draft':
      // A release that fails throws, and the hold stays pending so it can be
      // tried again; marking it approved with a failure note would be a
      // decision that did nothing, dressed as one that did.
      if (!deps.release) return 'recorded; nothing here can publish it'
      return deps.release(hold)
    default:
      return 'recorded'
  }
}
