import { estimateTokens } from '../telegram/transcript.js'
import type { ContextBlock } from './types.js'

export { estimateTokens }

/**
 * Keeps blocks, in order, until the budget is spent. The block that crosses
 * the line is cut at a line boundary rather than dropped, so a long lore
 * file still contributes its front; anything after it is left out.
 */
export function fit(blocks: ContextBlock[], budget: number): ContextBlock[] {
  const kept: ContextBlock[] = []
  let spent = 0
  for (const block of blocks) {
    if (spent + block.tokens <= budget) {
      kept.push(block)
      spent += block.tokens
      continue
    }
    const room = budget - spent
    if (room < 40) break
    const text = cutAtLine(block.text, room * 4)
    if (!text.trim()) break
    kept.push({ ...block, text, tokens: estimateTokens(text) })
    break
  }
  return kept
}

function cutAtLine(text: string, maxChars: number): string {
  if (text.length <= maxChars) return text
  const head = text.slice(0, maxChars)
  const line = head.lastIndexOf('\n')
  return (line > maxChars / 2 ? head.slice(0, line) : head).trimEnd()
}

/**
 * A small deterministic generator (mulberry32), so a run can be replayed
 * with the same draws: same memory probes, same forum sample, same tone.
 */
export function rng(seed: number): () => number {
  let state = seed >>> 0
  return () => {
    state = (state + 0x6d2b79f5) >>> 0
    let t = state
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

export function shuffle<T>(items: T[], random: () => number): T[] {
  const out = [...items]
  for (let i = out.length - 1; i > 0; i -= 1) {
    const j = Math.floor(random() * (i + 1))
    ;[out[i], out[j]] = [out[j] as T, out[i] as T]
  }
  return out
}

export function sample<T>(items: T[], count: number, random: () => number): T[] {
  return shuffle(items, random).slice(0, count)
}
