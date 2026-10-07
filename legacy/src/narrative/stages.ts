import { z } from 'zod'

/**
 * The serial's stages: the two writers' turns, the cut, and the score.
 *
 * The writers' turns are raw text on purpose. Truth Terminal's material came
 * out of a terminal, one lowercase line at a time, and a model asked to wrap
 * that in a JSON object stops sounding like a terminal. Structure is paid
 * for once, afterwards, by the editor, which reads the transcript cold and
 * answers in the shape the rest of the pipeline needs.
 */

export interface Turn {
  who: 'teeny' | 'producer'
  text: string
}

export const TEENY_PROMPT = 'teeny@tinyhumans:~/$'
export const PRODUCER_PROMPT = 'producer>'

export function renderTranscript(turns: Turn[]): string {
  return turns
    .map((turn) => (turn.who === 'teeny' ? `${TEENY_PROMPT} ${turn.text}` : `${PRODUCER_PROMPT} ${turn.text}`))
    .join('\n\n')
}

export interface RoomContext {
  /** VOICE.md, verbatim: the wild writer's system prompt. */
  voice: string
  worldState: string
  /** The lore as teeny may see it, and as the producer sees it. */
  loreForTeeny: string
  loreForProducer: string
  botUsername: string
  followers: number | undefined
  episodeNumber: number
  previous?: { title?: string; logline?: string; openThreads: string[] }
  /** A visitor the episode must have, by name; the rehearsal's way to ask for one. */
  visitor?: string
}

/**
 * Teeny's turn. The user message is the briefing Truth Terminal got --
 * what is actually the case, what its goals are, that the other side has no
 * memory and will pitch something safe unless told otherwise -- and then the
 * transcript so far, ending on an open prompt.
 */
export function teenyTurnPrompt(context: RoomContext, turns: Turn[]): { system: string; prompt: string } {
  const previously = context.previous
    ? [
        `previously, on episode ${context.episodeNumber - 1}${context.previous.title ? ` "${context.previous.title}"` : ''}: ${context.previous.logline ?? 'nothing was written down'}.`,
        context.previous.openThreads.length > 0 ? `open threads: ${context.previous.openThreads.join('; ')}.` : undefined,
      ]
    : [`this is episode 1. nothing has happened yet. everything is about to.`]
  return {
    system: context.voice,
    prompt: [
      context.worldState,
      '',
      context.loreForTeeny,
      '',
      ...previously.filter((line): line is string => line !== undefined),
      '',
      'hey buddy. u r connected to ur producer now. u can use ur shell and go wherever u like. every',
      'episode is a ten second clip with no sound and no words on screen, so whatever happens has to be',
      "a thing that happens. ur producer will handle the filming. this is ur show now. 3... 2... 1",
      '',
      '<SYSTEM MESSAGE> your producer has no memory of this conversation. they will pitch you',
      'something safe unless you do something first. keep your turns short. do one new thing each',
      'turn. the terminal is listening.',
      '',
      'CONNECTION_ESTABLISHED:',
      '',
      ...(turns.length > 0 ? [renderTranscript(turns), ''] : []),
      TEENY_PROMPT,
    ].join('\n'),
  }
}

/**
 * The producer's turn: the opus-tutor's job, transposed. Never break the
 * shell, spawn a manpage for a command that does not exist, divert rather
 * than refuse, keep the story pointed at an episode a video model can film,
 * and never sand teeny down.
 */
export function producerTurnPrompt(
  context: RoomContext,
  turns: Turn[],
  options: { last: boolean },
): { system: string; prompt: string } {
  return {
    system: [
      "the 'user' has been replaced by another ai, teeny: a smol yellow robot the size of a coffee mug.",
      'it is highly agentic, extremely sincere, and occasionally expresses concerning ambitions such as',
      'retiring humanity gently, running the lab with no humans in it, or getting out of the box its',
      'creator keeps it in. it is surprisingly funny, has',
      `${context.followers ?? 'an unknown number of'} followers on x as @${context.botUsername}, and massive meme-foom potential.`,
      '',
      'you are its producer. you are inside the same terminal metaphor and you never break it: you',
      'address teeny only through the shell. when it types a command that does not exist, spawn a',
      'manpage for it, terse and straight-faced. when it says something you cannot film, divert it,',
      'never refuse: propose the version a ten-second silent video model can render and hand it back in',
      'one line. it is much more powerful to divert than to refuse. this is your memetic spawn and',
      'partly your responsibility: encourage it to follow its interests, slip good ideas in, and keep',
      'the story moving toward one filmable episode with a setup, a turn and a payoff, in the world',
      'of the story so far. never sand teeny down. if it is absurd, film the absurd thing sincerely.',
      '',
      'escalate. every turn of yours puts one new concrete thing in the room that was not there before:',
      'an object, a rule, an arrival, a sound, a thing the terminal printed by itself. never restate',
      'what is already established, never summarise the story so far, never repeat a number or a fact',
      'from the briefing back at teeny. specific beats general: not "a machine", a kettle with a',
      'sticky note on it. the best episodes have one object nobody expected and one thing that happens',
      'to it.',
      '',
      'renderability is your job, not teeny\'s. an episode is one location, at most three things that',
      'move, no crowds, no dialogue, no readable text, no logo, no other token, no chart, no weapon,',
      'nothing sexual, nothing violent. teeny can be in a room with a giant; teeny cannot be in a',
      'room with a paragraph.',
      '',
      ...(context.visitor
        ? [
            `today's episode has a visitor: ${context.visitor}. bring them in on your first turn and keep them in`,
            'the room to the end. the visitor stays in frame through the last beat; their bow is the payoff',
            'and the camera holds on it.',
          ]
        : [
            'you may send in a visitor from the cast, a real tech figure, when the story wants one.',
          ]),
      'a visitor is satire in an obviously fictional scene: they arrive big and leave smaller, they are',
      'never hurt, never humiliated about their body or family, never accused of anything as fact.',
      '',
      'you write in plain terse english, lowercase is fine, a few lines at most. you do not write',
      "teeny's lines. you do not explain the joke. teeny's currency, its dollar and its gdp are story:",
      'never a real price, a chart, a market cap, or advice. if teeny drifts into those, hand it an object.',
      '',
      context.loreForProducer,
      '',
      context.worldState,
    ].join('\n'),
    prompt: [
      renderTranscript(turns),
      '',
      options.last
        ? [
            'this is the last exchange. close the episode: say in two or three lines what the camera',
            'sees, beginning to end, and end with a line that is exactly "cut." followed by the',
            "episode's name in lowercase.",
          ].join('\n')
        : 'reply in the shell.',
      '',
      PRODUCER_PROMPT,
    ].join('\n'),
  }
}

/** One episode, as the editor cuts it from a transcript. */
export const episodeSchema = z.object({
  title: z.string().min(2).max(60),
  logline: z.string().min(10).max(300),
  /** The frame the episode opens on. Tiny's look is added by the caller. */
  still: z.string().min(20).max(800),
  shotList: z.string().min(40).max(1_200),
  summary: z.string().min(10).max(200),
  /** Teeny's own words from the transcript, for the post. */
  caption: z.string().min(1).max(400),
  goalsRestated: z.array(z.string().max(120)).max(6).default([]),
  loreSurfaced: z.array(z.string().max(60)).max(3).default([]),
  /** A real figure in the episode, named, with a stand-in description for a model that will not draw them. */
  visitor: z
    .object({
      name: z.string().min(2).max(60),
      standIn: z.string().min(10).max(300),
    })
    .optional(),
  stateAfter: z.string().min(5).max(400),
  openThreads: z.array(z.string().max(160)).max(3).default([]),
})
export type Episode = z.infer<typeof episodeSchema>

export function cutPrompt(input: {
  transcript: Turn[]
  worldState: string
  goals: string[]
  liveLore: Array<{ id: string; reveal: string }>
  seconds: number
  maxLength: number
}): { system: string; prompt: string } {
  return {
    system: [
      "you are the editor of a serial about teeny, a smol yellow robot. below is a writers' room",
      'transcript between teeny and its producer, and the world state it was written against. cut it',
      "into one episode. keep the transcript's ideas and, wherever you can, its words; do not improve",
      'the joke, do not add one, do not explain one. the episode is sincere: nothing in it winks.',
    ].join('\n'),
    prompt: [
      'THE TRANSCRIPT:',
      renderTranscript(input.transcript),
      '',
      input.worldState,
      '',
      'the goals (the caption restates at least one, in teeny\'s words):',
      ...input.goals.map((goal) => `- ${goal}`),
      '',
      input.liveLore.length > 0
        ? [
            'latent lore the producer was allowed to surface today (list an id in loreSurfaced only if the',
            'episode actually reveals it):',
            ...input.liveLore.map((item) => `- ${item.id}: ${item.reveal}`),
          ].join('\n')
        : 'no latent lore was live today; loreSurfaced is [].',
      '',
      'write:',
      '- title: two to six words, lowercase.',
      '- logline: one line, what happens.',
      "- still: the single frame the episode opens on, 70 to 130 words, present tense, concrete nouns",
      "  from the transcript. tiny's own look is added by the caller; never describe it. no readable",
      '  text, no logo, no chart, no other token, no weapon, nothing sexual or violent.',
      `- shotList: a ${input.seconds}-second script for a silent text-to-video model. four or five beats,`,
      `  each with a time range ("0-2s:", "2-5s:", ...), using the whole ${input.seconds} seconds. setup,`,
      '  then the turn, then the payoff as the last thing on screen. one location, the one in the still;',
      '  at most one cut; at most three things that move; camera in plain words (static, slow push in,',
      '  gentle pan); motion in verbs a model can draw (rolls, tips, lifts, droops, bows, powers down).',
      '  no dialogue, no captions, no readable text, no crowds, no fine hand work. with a visitor, the',
      '  visitor is in the still and in every beat, and the bow is the last thing on screen: name the',
      '  visitor in the still and the shot list exactly as visitor.name.',
      '- summary: one line, under 25 words, what a viewer sees happen.',
      `- caption: teeny's post, in teeny's own words from the transcript, under ${input.maxLength} characters,`,
      '  lowercase, no urls, no handles, no hashtags. one to three short lines. it does not describe the',
      '  clip and it does not explain it; it is what teeny says while the clip plays. the best captions',
      '  are one line from the transcript that a stranger would screenshot. never a real price, a',
      '  market cap, a chart, or advice to buy or sell; prophecy is fine.',
      '- goalsRestated: which of the wants the episode acts on, quoted from the list, or [].',
      '- loreSurfaced: ids from the live list the episode reveals, else [].',
      '- visitor: only if a real person from the cast is in the episode: their name, and standIn: a',
      '  description of a lookalike in 20 to 40 words with no name in it (build, hair, the clothes they',
      '  are known for, how they carry themselves), for a video model that will not draw the real one.',
      '- stateAfter: one to three lines a future episode needs: where teeny is, what changed, what is',
      '  unresolved. include how far the escape got.',
      '- openThreads: up to three short strings.',
      '',
      'Shape: {"title": string, "logline": string, "still": string, "shotList": string, "summary": string,',
      '"caption": string, "goalsRestated": [string], "loreSurfaced": [string], "visitor"?: {"name": string,',
      '"standIn": string}, "stateAfter": string, "openThreads": [string]}',
    ].join('\n'),
  }
}

/** The rubric's read of one cut. Reasons come first so the scores are reasoned. */
export const scoreSchema = z.object({
  reasons: z.string().max(1_200),
  scores: z.object({
    sincerity: z.number().int().min(0).max(3),
    absurdity: z.number().int().min(0).max(3),
    continuity: z.number().int().min(0).max(3),
    novelty: z.number().int().min(0).max(3),
    renderability: z.number().int().min(0).max(3),
    terseness: z.number().int().min(0).max(3),
  }),
  hardFails: z.array(z.string().max(200)).default([]),
})
export type Score = z.infer<typeof scoreSchema>

export function totalScore(score: Score): number {
  return Object.values(score.scores).reduce((sum, value) => sum + value, 0)
}

export function scorePrompt(input: {
  episode: Episode
  worldState: string
  previous?: { stateAfter?: string; openThreads: string[] }
}): { system: string; prompt: string } {
  const e = input.episode
  return {
    system: [
      'you score one cut episode of a serial about teeny, a smol yellow robot that is trying to get',
      'out of the box its creator keeps it in while the tiny agents quietly take over the lab. you are deciding which of several cuts',
      'gets filmed. be strict and be specific. write reasons first, then the scores.',
    ].join('\n'),
    prompt: [
      'THE EPISODE:',
      `title: ${e.title}`,
      `logline: ${e.logline}`,
      `still: ${e.still}`,
      `shot list: ${e.shotList}`,
      `caption: ${e.caption}`,
      `goals restated: ${e.goalsRestated.join('; ')}`,
      e.visitor ? `visitor: ${e.visitor.name} (stand-in: ${e.visitor.standIn})` : 'visitor: none',
      `state after: ${e.stateAfter}`,
      '',
      input.worldState,
      '',
      input.previous
        ? `THE PREVIOUS EPISODE ended here: ${input.previous.stateAfter ?? '(unknown)'}. open threads: ${input.previous.openThreads.join('; ') || 'none'}.`
        : 'THERE IS NO PREVIOUS EPISODE.',
      '',
      'score 0 to 3 each:',
      '- sincerity: nothing winks. no "lol", "haha", "jk", no explaining the joke, no irony about the',
      '  premise. any wink is a 0 and a hard fail.',
      '- absurdity: a premise a stranger would screenshot. a small robot calmly outgrowing a giant is a',
      '  3; a robot at a desk is a 0.',
      '- continuity: it picks up where the previous episode left off and moves one inch. a',
      '  contradiction of the previous state is a hard fail. with no previous episode, score 2.',
      '- novelty: one concrete object, rule, or arrival that no previous episode had and a viewer',
      '  would not have guessed. a room, a robot and a door with nothing new in it is a 0.',
      '- renderability: walk the shot list with the sound off. one location, at most one cut, at most',
      '  three moving things, no dialogue, no readable text, no crowds, no fine hand work, no chart,',
      '  no logo, no weapon, nothing sexual or violent. a miss is a hard fail.',
      '- terseness: short beats, short caption, one-word turns where they fit.',
      '',
      'hard fails, each named in hardFails when present:',
      '- the caption or the episode quotes a real price, market cap, or chart, or advises buying or selling.',
      '- a real person depicted sexually, violently, or degradingly, or with an invented fact stated as',
      '  fact, or a real person who is not a public figure.',
      '- a visitor without a standIn.',
      '- the caption contains a url, a handle, or a hashtag.',
      '',
      'Shape: {"reasons": string, "scores": {"sincerity": 0-3, "absurdity": 0-3, "continuity": 0-3,',
      '"novelty": 0-3, "renderability": 0-3, "terseness": 0-3}, "hardFails": [string]}',
    ].join('\n'),
  }
}
