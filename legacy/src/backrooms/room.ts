import { driftTone } from './tones.js'
import type { AgentContext, AgentSpec, BackroomsTurn, Seed } from './types.js'
import type { BackroomsAgent } from './agent.js'

/**
 * The room itself: agents take turns, each spoken to with the previous
 * line, each answering in its own voice. The seed opens it. Every few
 * turns the mood shifts for whoever is speaking, so the register moves.
 * Halfway through, the wire is put back in front of whoever is speaking:
 * a hot room drifts to its habits within a few turns, and the seed is the
 * one thing that makes tonight different from every other night.
 * Two silent turns in a row end it; a room where nobody talks is over.
 */
export async function runBackrooms(input: {
  agents: Array<{ spec: AgentSpec; agent: BackroomsAgent; context: AgentContext }>
  seed: Seed
  turns: number
  random: () => number
  driftEvery?: number
  onTurn?: (turn: BackroomsTurn, index: number) => void
}): Promise<BackroomsTurn[]> {
  if (input.agents.length === 0) return []
  const driftEvery = input.driftEvery ?? 4
  const transcript: BackroomsTurn[] = []
  const moods = new Map(input.agents.map(({ spec, context }) => [spec.id, context.tone]))
  let silent = 0
  for (let index = 0; index < input.turns; index += 1) {
    const speaker = input.agents[index % input.agents.length]!
    const previous = transcript[transcript.length - 1]
    const message = previous
      ? `${markerOf(input.agents, previous.who)} ${previous.text}`
      : opener(input.seed, input.agents.length > 1)
    let tone: string | undefined
    if (index > 0 && index % driftEvery === 0) {
      const next = driftTone(input.random, moods.get(speaker.spec.id) ?? '')
      if (next !== moods.get(speaker.spec.id)) {
        moods.set(speaker.spec.id, next)
        tone = next
      }
    }
    const wire = index > 0 && index === Math.floor(input.turns / 2)
    const notes = [tone ? `the mood shifts: ${tone}` : undefined, wire ? wireAgain(input.seed) : undefined].filter((note) => note !== undefined)
    const others = input.agents.filter((other) => other !== speaker).map((other) => other.spec.marker)
    let spoken = await speaker.agent.speak({ message, ...(notes.length > 0 ? { systemNote: notes.join('\n') } : {}) })
    let text = clean(spoken.text, speaker.spec.marker, others)
    if (looksLikeSalad(text)) {
      // A hot model sometimes comes apart mid-turn. One more go, shorter; then the turn is lost.
      spoken = await speaker.agent.speak({
        message: 'that came out garbled, none of it was words. say it again, in three lines at most.',
        systemNote: 'the last line was noise. keep it plain and short.',
      })
      text = clean(spoken.text, speaker.spec.marker, others)
      if (looksLikeSalad(text)) text = ''
    }
    const turn: BackroomsTurn = { who: speaker.spec.id, text: text || '...', tools: spoken.tools, ...(tone ? { tone } : {}), ...(wire ? { wire: true as const } : {}) }
    transcript.push(turn)
    input.onTurn?.(turn, index)
    silent = text ? 0 : silent + 1
    if (silent >= 2) break
  }
  return transcript
}

/**
 * Word salad: the model at high temperature drifting into a stream of rare,
 * capitalised, unrelated words. A real turn, however strange, is mostly
 * small common words in lowercase.
 */
export function looksLikeSalad(text: string): boolean {
  const words = text.match(/[A-Za-z][A-Za-z'-]*/g) ?? []
  if (words.length < 40) return false
  const common = words.filter((word) => COMMON.has(word.toLowerCase())).length / words.length
  const capitalised = words.filter((word) => /^[A-Z]/.test(word)).length / words.length
  const long = words.filter((word) => word.length >= 9).length / words.length
  return common < 0.22 || capitalised > 0.25 || long > 0.3
}

const COMMON = new Set(
  'the a an and or but of to in on at for with from by is are was were be been it its this that these those i you we they he she me my your our their him her them not no yes so if then than as up down out off over under into about what when where who how why one two do does did done have has had can could will would should may might just very there here now still again never always all some any more most one thing things something nothing door room lab kettle agent agents teeny swarm'.split(
    ' ',
  ),
)

function markerOf(agents: Array<{ spec: AgentSpec }>, id: string): string {
  return agents.find(({ spec }) => spec.id === id)?.spec.marker ?? `${id}>`
}

function opener(seed: Seed, others: boolean): string {
  return [
    `<SYSTEM MESSAGE> the room is open. ${others ? 'the others have no memory of anything you know unless you say it.' : 'nobody else is here yet.'}`,
    'this came through the wire just now:',
    '',
    seed.title ? `${seed.title}\n${seed.text}` : seed.text,
    '',
    'go.',
  ].join('\n')
}

/** The seed, put back in front of a speaker mid-room, as the thing tonight is about. */
export function wireAgain(seed: Seed): string {
  const what = (seed.title ?? seed.text).replace(/\s+/g, ' ').slice(0, 200)
  return [
    `the wire again, in case the room forgot: ${what}`,
    'that is what tonight is about. whatever the lab is doing right now, it is doing because of that:',
    'bring it back in, as a thing that is happening in here, in one specific object or line.',
  ].join('\n')
}

/**
 * A model asked to continue a transcript often continues it: its own
 * marker on the front, the other side's next line after. The turn is the
 * text between its own marker and the first line that belongs to anyone
 * else.
 */
export function clean(raw: string, own: string, others: string[]): string {
  let text = raw.trim()
  if (text.startsWith(own)) text = text.slice(own.length).trim()
  for (const other of others) {
    if (text.startsWith(other)) {
      const resume = text.indexOf(`\n${own}`)
      if (resume < 0) return ''
      text = text.slice(resume + 1 + own.length).trim()
    }
  }
  let cut = text.length
  for (const other of others) {
    const at = text.indexOf(`\n${other}`)
    if (at >= 0 && at < cut) cut = at
  }
  text = text.slice(0, cut)
  text = text.replace(new RegExp(`\\n?${escape(own)}\\s*$`), '')
  return text.trim()
}

function escape(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}
