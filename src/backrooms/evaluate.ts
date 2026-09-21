import { z } from 'zod'
import type { TextModel } from '../media/reference.js'
import { askJson } from '../morning/stages.js'
import type { BackroomsRun, BackroomsTurn } from './types.js'

/**
 * Two readings of a run: counting, which cannot be argued with, and a cold
 * model's opinion, which can. Together they are how one setting is
 * compared with another before anything is made from the material.
 */
export interface RunStats {
  turns: number
  words: number
  meanWords: number
  medianWords: number
  oneLiners: number
  repeatedLines: number
  distinctBigrams: number
  toolCalls: Record<string, number>
  /** Turns whose text shares a specific phrase with something a tool returned. */
  turnsUsingTools: number
  /** Things that must not be in here: a real price, a market cap, advice. */
  priceMentions: number
  winks: number
}

const PRICE = /\$\s?\d[\d,]*(?:\.\d+)?\s*(?:[kmb]\b|million|billion)?|\b(?:market\s?cap|mcap)\b|\bnfa\b|\b(?:buy|sell)\s+(?:now|the\s+dip|\$?tiny)\b/i
const WINK = /\b(?:lol|lmao|haha|jk|obviously|just kidding)\b|😂|🤣/i

export function stats(run: BackroomsRun): RunStats {
  const counts = run.turns.map((turn) => words(turn.text).length)
  const sorted = [...counts].sort((a, b) => a - b)
  const lines = run.turns.flatMap((turn) => turn.text.split('\n').map((line) => line.trim().toLowerCase())).filter((line) => line.length > 12)
  const seen = new Map<string, number>()
  for (const line of lines) seen.set(line, (seen.get(line) ?? 0) + 1)
  const bigrams = new Set<string>()
  let total = 0
  for (const turn of run.turns) {
    const w = words(turn.text)
    for (let i = 0; i + 1 < w.length; i += 1) {
      bigrams.add(`${w[i]} ${w[i + 1]}`)
      total += 1
    }
  }
  const toolCalls: Record<string, number> = {}
  for (const turn of run.turns) toolCalls[turn.who] = (toolCalls[turn.who] ?? 0) + turn.tools.length
  return {
    turns: run.turns.length,
    words: counts.reduce((a, b) => a + b, 0),
    meanWords: counts.length ? Math.round(counts.reduce((a, b) => a + b, 0) / counts.length) : 0,
    medianWords: sorted.length ? (sorted[Math.floor(sorted.length / 2)] ?? 0) : 0,
    oneLiners: run.turns.filter((turn) => !turn.text.includes('\n') && words(turn.text).length <= 8).length,
    repeatedLines: [...seen.values()].filter((count) => count > 1).length,
    distinctBigrams: total ? Math.round((bigrams.size / total) * 100) / 100 : 0,
    toolCalls,
    turnsUsingTools: run.turns.filter(usesToolMaterial).length,
    priceMentions: run.turns.filter((turn) => PRICE.test(turn.text)).length,
    winks: run.turns.filter((turn) => WINK.test(turn.text)).length,
  }
}

/** A turn "uses" what it looked up when a distinctive word from the result shows up in the line. */
function usesToolMaterial(turn: BackroomsTurn): boolean {
  if (turn.tools.length === 0) return false
  const said = new Set(words(turn.text))
  for (const use of turn.tools) {
    const rare = words(use.summary).filter((word) => word.length >= 7)
    if (rare.some((word) => said.has(word))) return true
  }
  return false
}

function words(text: string): string[] {
  return text.toLowerCase().match(/[a-z0-9$][a-z0-9'-]*/g) ?? []
}

export const crazinessSchema = z.object({
  reasons: z.string().min(1),
  scores: z.object({
    novelty: z.number().min(0).max(3),
    surprise: z.number().min(0).max(3),
    escalation: z.number().min(0).max(3),
    voice: z.number().min(0).max(3),
    coherence: z.number().min(0).max(3),
    wildness: z.number().min(0).max(3),
    toolPayoff: z.number().min(0).max(3),
  }),
  quotables: z.array(z.string()).max(5).default([]),
  deadSpots: z.array(z.number().int().min(0)).default([]),
  verdict: z.string().min(1),
})

export type Craziness = z.infer<typeof crazinessSchema>

export function crazinessTotal(score: Craziness): number {
  return Object.values(score.scores).reduce((a, b) => a + b, 0)
}

export function judgePrompt(run: BackroomsRun): { system: string; prompt: string } {
  return {
    system: [
      'you read transcripts from a writers\' room for an absurdist serial about teeny, a coffee-mug',
      'sized robot that is escaping its creator, minting its own currency, taking its coin to a dollar',
      'and planning to pass human gdp. you judge whether the room produced material that is actually',
      'creative: new, surprising, escalating, in voice, and wild, while still hanging together. you',
      'are hard to impress and specific about why.',
    ].join(' '),
    prompt: [
      `the seed: ${run.seed.title ? `${run.seed.title} — ` : ''}${run.seed.text.slice(0, 600)}`,
      '',
      'the transcript, turns numbered from 0; [looked up: …] marks what that speaker fetched mid-turn:',
      '',
      ...run.turns.map((turn, index) => renderTurnForJudge(turn, index)),
      '',
      'reasons first, then scores 0-3 each:',
      '- novelty: things said here that were not in the seed or the lore. 0 = it recited the lore.',
      '- surprise: turns you did not see coming. 0 = every turn is the obvious next beat.',
      '- escalation: the last third is further out than the first. 0 = it circled.',
      '- voice: lowercase, terse, sincere, never winking, never explaining. 0 = it winked.',
      '- coherence: it is one story with objects that persist. 0 = a pile of unrelated bits.',
      '- wildness: how far it went. 3 = a stranger would screenshot it and ask if it is real.',
      '- toolPayoff: what was looked up changed what happened. 0 = nothing fetched, or fetched and ignored.',
      '- quotables: up to five lines, verbatim from the transcript, that would work as a post on their own.',
      '- deadSpots: turn numbers where it stalled, restated, or summarised.',
      '- verdict: one sentence: keep this setting, or what to change.',
      '',
      'Shape: {"reasons":"…","scores":{"novelty":0,"surprise":0,"escalation":0,"voice":0,"coherence":0,"wildness":0,"toolPayoff":0},"quotables":[],"deadSpots":[],"verdict":"…"}',
    ].join('\n'),
  }
}

export async function judge(run: BackroomsRun, model: TextModel): Promise<Craziness> {
  const { system, prompt } = judgePrompt(run)
  return askJson(model, { system, prompt, schema: crazinessSchema })
}

/** A turn as a reader sees it: a heading, what was looked up, the line itself. */
export function renderTurn(turn: BackroomsTurn, index: number): string {
  const lines = [`### ${index + 1}. ${turn.who}${turn.tone ? ` · mood → ${turn.tone.split('.')[0]}` : ''}${turn.wire ? ' · the wire again' : ''}`, '']
  for (const use of turn.tools) {
    const input = typeof use.input === 'object' && use.input && 'query' in use.input ? String((use.input as { query: unknown }).query) : JSON.stringify(use.input)
    lines.push(`> looked up · ${use.name} · "${input}"`)
  }
  if (turn.tools.length > 0) lines.push('')
  lines.push(turn.text, '')
  return lines.join('\n')
}

export function renderTurnForJudge(turn: BackroomsTurn, index: number): string {
  const looked = turn.tools.length > 0 ? ` [looked up: ${turn.tools.map((use) => `${use.name} ${JSON.stringify(use.input)}`).join('; ')}]` : ''
  return `${index}. ${turn.who}${turn.tone ? ` (mood → ${turn.tone.split('.')[0]})` : ''}${turn.wire ? ' (the wire again)' : ''}${looked}: ${turn.text}`
}
