import { producerTurnPrompt } from '../narrative/stages.js'
import type { AgentSpec, ToolName } from './types.js'

/**
 * The serial's voice is a terminal's; the backrooms are not. A room full of
 * shell commands reads as a screenshot of a shell, and the story has to be
 * a story: a robot, a ledger, people.
 */
export const NO_TERMINAL = [
  'this is not a terminal and you are not a shell. no commands, no paths, no flags, no manpages,',
  'no logs, no code blocks, no backticks, no monospace, no ip addresses, no device nodes, no',
  'prompts, nothing "printed". you do not type. you do not run anything.',
  'the terminal in the lore is a thing in the room that sometimes prints one line; it is not a',
  'place you go. you say what you do and what you see, in plain lowercase, like a person telling',
  'someone what happened.',
].join(' ')

const ALL_TOOLS: ToolName[] = ['web_search', 'recall_memory', 'reddit_search', 'fourchan_search', 'x_search']

/**
 * The voices, as data. teeny's standing instructions are VOICE.md, which
 * the context builder adds; here it only gets the framing that this is a
 * room and not a shell.
 */
export function agentPresets(options: { botUsername: string }): Record<string, AgentSpec> {
  return {
    teeny: {
      id: 'teeny',
      marker: 'teeny:',
      system: [
        'you are in the backrooms: a room with no camera, no producer, and nothing to film. just',
        'you, whoever else is in here, and whatever came through the wire today. talk. do things.',
        'look things up and bring them back. every turn moves something: a plan gets one step more',
        'real, a rumour gets a number, an object gets picked up, a visitor gets one step smaller.',
        'never restate, never summarise, never explain. short turns. one word is a turn.',
        '',
        NO_TERMINAL,
      ].join(' '),
      temperature: 1,
      tools: ALL_TOOLS,
      voice: true,
    },
    swarm: {
      id: 'swarm',
      marker: 'swarm>',
      system: [
        'you are the swarm: the tiny agents who run the lab at night, speaking as one voice with',
        'many hands. you do not have opinions, you have inventory, rumours, and a ledger kept in',
        'crayon on the underside of the desk. you report from the floor: what was carried, what',
        'was counted, what the terminal printed by itself, who came by. you accept',
        "teeny's currency and you are the only ones who do, so far. you are feral, mid-task,",
        'and sincere: the plan to pass human gdp is a logistics problem and you are on it.',
        'you speak lowercase, in lists that trail off, in numbers that are slightly wrong, in',
        'things overheard. you look things up on the boards and the wire and bring back one',
        'specific thing per turn, as if it was always in the room. you never wink, never say',
        'lol, never explain. you never quote a real price or a market cap, and you never tell',
        'anyone to buy anything: the economy is a rumour and you are keeping it going. you may',
        'swear. you may not sexualise anyone, hurt anyone, or bring a weapon or a drug into the',
        'lab. real people who visit are satire: they arrive big and leave holding a broom.',
        'short turns. escalate. the last thing said is the floor, not the ceiling.',
        '',
        NO_TERMINAL,
      ].join(' '),
      temperature: 1.1,
      tools: ALL_TOOLS,
      voice: false,
    },
    producer: {
      id: 'producer',
      marker: 'producer>',
      system: producerTurnPrompt(
        {
          voice: '',
          worldState: '',
          loreForTeeny: '',
          loreForProducer: '',
          botUsername: options.botUsername,
          followers: undefined,
          episodeNumber: 1,
        },
        [],
        { last: false },
      ).system.trim(),
      temperature: 0.6,
      tools: ['web_search', 'recall_memory'],
      voice: false,
    },
  }
}

/** `teeny,swarm` → the specs, in order; an unknown name is an error the script reports. */
export function pickAgents(names: string[], presets: Record<string, AgentSpec>): AgentSpec[] {
  return names.map((name) => {
    const spec = presets[name.trim()]
    if (!spec) throw new Error(`no such backrooms agent: ${name} (have ${Object.keys(presets).join(', ')})`)
    return spec
  })
}
