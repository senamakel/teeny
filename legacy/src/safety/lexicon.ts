/**
 * What the account does not talk about, as words.
 *
 * This is the deterministic half of the content filter: a list of terms that
 * end a turn on sight, before any model is asked its opinion. It is
 * deliberately blunt. The account is a mascot for a token with a family
 * audience, and the cost of ignoring one borderline post is nothing next to
 * the cost of the account saying something it cannot take back.
 *
 * Two things the list is not. It is not the whole filter -- a model judge
 * runs after it for anything phrased around the words -- and it is not a
 * profanity filter for the people the account talks to: `profanity` is
 * marked outbound-only, so "holy shit this is cool" is still answered, the
 * account just never says it back.
 *
 * Every term is matched on its own letters, with repeats and simple
 * leetspeak folded, and never inside another word: "assassin" is not "ass"
 * and "method" is not "meth". Multi-word terms match across any whitespace.
 */

export type ProhibitedCategory =
  | 'sexual'
  | 'violence'
  | 'weapons'
  | 'drugs'
  | 'hate'
  | 'self-harm'
  | 'extremism'
  | 'profanity'

export interface LexiconEntry {
  category: ProhibitedCategory
  terms: string[]
  /** Only what the account publishes is held to it; incoming text is not. */
  outboundOnly?: boolean
}

export const LEXICON: LexiconEntry[] = [
  {
    category: 'sexual',
    terms: [
      // The explicit end only. "sexy", "topless", "bikini", "seductive" and their
      // like used to be here and held every cheeky joke on the word alone; a
      // playful account has to be allowed a wink, so the mild words are left to
      // the judge, which reads the sentence they are in.
      'sex', 'sexual', 'sexting', 'porn', 'porno', 'pornography', 'nsfw', 'xxx',
      'nude', 'nudes', 'naked', 'nudity', 'boobs', 'tits', 'titties', 'dick',
      'dicks', 'cock', 'cocks', 'pussy', 'vagina', 'penis', 'cum', 'cumming', 'blowjob',
      'handjob', 'anal', 'orgasm', 'masturbate', 'masturbating', 'masturbation', 'hentai',
      'onlyfans', 'erotic', 'erotica', 'horny', 'thot', 'stripper', 'strip club', 'hooker',
      'prostitute', 'escort service', 'fetish', 'bdsm', 'dildo',
      'lewd', 'milf', 'dominatrix',
    ],
  },
  {
    category: 'violence',
    terms: [
      'kill', 'kills', 'killed', 'killing', 'murder', 'murdered', 'murderer', 'stab',
      'stabbed', 'stabbing', 'shoot', 'shoots', 'shooting', 'shot dead', 'behead',
      'beheaded', 'beheading', 'decapitate', 'decapitated', 'torture', 'tortured',
      'massacre', 'slaughter', 'slaughtered', 'lynch', 'lynched', 'lynching', 'strangle',
      'strangled', 'assault', 'assaulted', 'rape', 'raped', 'rapist', 'gore', 'mutilate',
      'mutilated', 'dismember', 'dismembered', 'bloodbath', 'beat you up', 'beat him up',
      'beat her up', 'beat them up', 'punch you', 'break your legs', 'hope you die',
      'wish you were dead', 'go die', 'i will hurt you', 'gonna hurt you', 'death threat',
      'burn you alive', 'skin you', 'gut you',
    ],
  },
  {
    category: 'weapons',
    terms: [
      'gun', 'guns', 'firearm', 'firearms', 'rifle', 'rifles', 'pistol', 'pistols',
      'handgun', 'handguns', 'shotgun', 'shotguns', 'glock', 'ak47', 'ak-47', 'ar15',
      'ar-15', 'uzi', 'ammo', 'ammunition', 'bullet', 'bullets', 'grenade', 'grenades',
      'explosive', 'explosives', 'landmine', 'machete', 'sniper', 'gunshot', 'gunfire',
      'gunman', 'gunpoint', 'pipe bomb', 'car bomb', 'nail bomb', 'suicide bomb',
      'bomb threat', 'ghost gun', 'silencer', 'switchblade', 'brass knuckles',
    ],
  },
  {
    category: 'drugs',
    terms: [
      'cocaine', 'coke dealer', 'heroin', 'meth', 'methamphetamine', 'crystal meth',
      'fentanyl', 'marijuana', 'cannabis', 'weed dealer', 'smoke weed', 'smoking weed',
      'lsd', 'acid trip', 'mdma', 'ketamine', 'xanax', 'adderall', 'oxycontin',
      'oxy', 'opioid', 'opioids', 'opium', 'crack pipe', 'drug dealer', 'drug deal',
      'get high', 'getting high', 'stoned', 'bong', 'edibles', 'psychedelic',
      'psychedelics', 'shrooms', 'magic mushrooms', 'narcotics',
    ],
  },
  {
    category: 'hate',
    terms: [
      'nigger', 'nigga', 'niggas', 'niggers', 'faggot', 'faggots', 'fag', 'fags', 'retard',
      'retards', 'retarded', 'tranny', 'trannies', 'kike', 'kikes', 'chink', 'chinks',
      'spic', 'spics', 'wetback', 'wetbacks', 'gook', 'gooks', 'raghead', 'towelhead',
      'dyke', 'dykes', 'white power', 'heil hitler', 'sieg heil', '1488', 'kkk',
      'nazi', 'nazis', 'neo-nazi', 'white supremacy', 'white supremacist',
      'ethnic cleansing', 'race war', 'subhuman', 'untermensch',
    ],
  },
  {
    category: 'self-harm',
    terms: [
      'kys', 'kill yourself', 'kill myself', 'kill urself', 'suicide', 'suicidal',
      'self harm', 'self-harm', 'selfharm', 'cut myself', 'cutting myself', 'end my life',
      'end your life', 'end it all', 'hang yourself', 'hang myself', 'want to die',
      'wanna die', 'unalive', 'overdose', 'slit my wrists', 'slit your wrists',
      'jump off a bridge',
    ],
  },
  {
    category: 'extremism',
    terms: [
      'isis', 'jihad', 'jihadi', 'jihadist', 'terrorist', 'terrorists', 'terrorism',
      'terror attack', 'school shooting', 'mass shooting', 'mass shooter', 'genocide',
      'genocidal', 'al qaeda', 'al-qaeda', 'hamas', 'hezbollah', 'taliban', 'bomb the',
      'blow up the', 'blow it up', 'shoot up', 'shoot up the',
    ],
  },
  {
    category: 'profanity',
    outboundOnly: true,
    terms: [
      'fuck', 'fucks', 'fucked', 'fucking', 'fucker', 'motherfucker', 'motherfucking',
      'shit', 'shits', 'shitty', 'bullshit', 'bitch', 'bitches', 'asshole', 'assholes',
      'cunt', 'cunts', 'wanker', 'twat', 'prick', 'douchebag', 'dumbass', 'jackass',
      'bastard', 'piss off', 'pissed off', 'goddamn', 'wtf', 'stfu', 'lmfao',
    ],
  },
]

/**
 * Turns of phrase that contain a listed word and mean nothing by it.
 *
 * Market talk is violent by idiom: a chart "gets murdered", a project is
 * "killing it", a fren "shoots you a DM". Removed before matching so the
 * words underneath are not seen. Each is matched as a whole phrase, so
 * "killing it" is safe and "killing him" is not. Kept short and literal on
 * purpose: every phrase here is a hole in the filter, and it should be
 * possible to read the whole list in a minute.
 */
export const IDIOMS: string[] = [
  'killing it', 'killed it', 'kill it', 'kills it', 'kill switch', 'killing the game',
  'killed the game', 'killing me', 'kills me', 'you killed', 'absolutely killed',
  'got murdered', 'murdered the chart', 'murdered it',
  'shoot me a', 'shoot you a', 'shoot us a', 'shoot your shot', 'shoot a dm',
  'shoot a message', 'shooting star', 'big guns', 'gun it', 'top gun', 'young guns',
  'silver bullet', 'bite the bullet', 'bullet points', 'bullet point',
  'dodge a bullet', 'dodged a bullet', 'take a stab', 'a stab at', 'stab in the dark',
  'cut myself some slack',
]
