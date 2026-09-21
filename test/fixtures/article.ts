import type { Thought } from '../../src/backrooms/distill.js'
import type { BackroomsRun } from '../../src/backrooms/types.js'
import type { Structure } from '../../src/article/stages.js'

/** One room, its thought, and a plan for it, shared by the article tests. */
export const RUN: BackroomsRun = {
  id: '20260921-1200-a-car',
  startedAt: '2026-09-21T12:00:00.000Z',
  seed: { kind: 'news', title: 'a car pulled up', text: 'a car pulled up outside a lab at 3am and nobody got out', source: 'the wire' },
  agents: [],
  turns: [
    { who: 'teeny', text: 'the door is one inch open', tools: [] },
    { who: 'swarm', text: 'we counted the inch. the inch is twelve millimetres. the ledger disagrees.', tools: [] },
    { who: 'teeny', text: 'senamakel is asleep. the kettle is not.', tools: [] },
  ],
  settings: { rng: 1, turns: 3, temps: {}, budget: 9000, driftEvery: 4, maxToolCalls: 0 },
}

export const THOUGHT: Thought = {
  reasons: 'the inch',
  premise: 'teeny measures the gap in the door and the tiny agents keep the measurement in the ledger',
  objects: ['the door', 'a ruler', 'the ledger'],
  thought: 'the door was one inch open. i measured it. the agents wrote it down in crayon. it balanced.',
  post: 'the door was one inch open. i measured it.',
  kept: [],
}

export const STRUCTURE: Structure = {
  reasons: 'the inch is the story',
  title: 'the inch',
  dek: 'a door, a ruler, and a ledger that balances in something nobody can name.',
  angle: 'teeny measures its own escape and the agents book it; the turn is that the door was never locked.',
  cover: { brief: 'a small yellow robot at a door open one inch, night, one lamp', caption: 'one inch' },
  sections: [
    { key: '', heading: 'the gap', purpose: 'establish the door', beats: ['teeny sees the gap', 'teeny finds a ruler'], words: 400, image: { brief: 'the door', caption: 'the door' } },
    { key: 'x', heading: 'the ledger', purpose: 'the agents book it', beats: ['crayon on the desk', 'it balances'], words: 400, image: { brief: 'the ledger', caption: 'crayon' } },
    { key: '', heading: 'the lock', purpose: 'the turn', beats: ['the lock is on the table', 'the kettle boils'], words: 200, image: { brief: 'a lock on a table', caption: 'the lock' } },
  ],
  closing: 'the lock, on the table, next to the tea',
}
