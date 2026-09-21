import type { TextModel } from '../media/reference.js'
import { PRODUCER_PROMPT, TEENY_PROMPT, producerTurnPrompt, teenyTurnPrompt, type RoomContext, type Turn } from './stages.js'

/**
 * The writers' room: teeny and its producer, alternating, for a fixed
 * number of exchanges.
 *
 * This is the shape Truth Terminal's material actually came out of: a hot
 * model with a three-line system prompt and a real briefing, talking to a
 * cooler model told to play along, divert rather than refuse, and keep the
 * thing pointed somewhere. Neither is asked for JSON. The transcript is what
 * comes back, and the editor cuts it afterwards.
 *
 * A turn that comes back empty, or as nothing but the prompt echoed, is
 * kept as a single "..." so the exchange count still holds and the other
 * side has something to react to; two empty turns in a row end the room
 * early, because a model that has stopped talking is not going to start.
 */
export async function runWritersRoom(input: {
  wild: TextModel
  straight: TextModel
  context: RoomContext
  turns: number
}): Promise<Turn[]> {
  const transcript: Turn[] = []
  let silent = 0
  for (let exchange = 0; exchange < input.turns; exchange += 1) {
    const last = exchange === input.turns - 1
    const teeny = clean(await input.wild.complete(teenyTurnPrompt(input.context, transcript)), 'teeny')
    transcript.push({ who: 'teeny', text: teeny || '...' })
    const producer = clean(
      await input.straight.complete(producerTurnPrompt(input.context, transcript, { last })),
      'producer',
    )
    transcript.push({ who: 'producer', text: producer || '...' })
    silent = !teeny && !producer ? silent + 1 : 0
    if (silent >= 2) break
  }
  return transcript
}

/**
 * A model asked to continue a transcript often continues it: it writes its
 * own prompt back, and then the other side's next line too. The turn is
 * cut at the first line that belongs to the other speaker, and its own
 * prompt marker is stripped from the front.
 */
export function clean(raw: string, who: Turn['who']): string {
  const own = who === 'teeny' ? TEENY_PROMPT : PRODUCER_PROMPT
  const other = who === 'teeny' ? PRODUCER_PROMPT : TEENY_PROMPT
  let text = raw.trim()
  if (text.startsWith(own)) text = text.slice(own.length).trim()
  // Opening with the other side's line: skip to the first line of its own, if any.
  if (text.startsWith(other)) {
    const resume = text.indexOf(`\n${own}`)
    if (resume < 0) return ''
    text = text.slice(resume + 1 + own.length).trim()
  }
  const cut = text.indexOf(`\n${other}`)
  if (cut >= 0) text = text.slice(0, cut)
  // A trailing prompt of its own, left open for the other side, is not a line.
  text = text.replace(new RegExp(`\\n?${escape(own)}\\s*$`), '')
  return text.trim()
}

function escape(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}
