import { z } from 'zod'
import { compactUsd, price } from '../chart/format.js'
import type { Rung } from './ladder.js'
import type { ListedCoin } from './leaderboard.js'
import { multiple } from './svg.js'

export const moonCaptionSchema = z.object({
  text: z.string().min(1).max(400),
})

export interface MoonCaptionInput {
  token: ListedCoin
  target: Rung
  ladder: Rung[]
  boardName: string
  /** What the account already posted in this series, from memory, untrusted. */
  history: string
  community: string
  maxEmoji: number
  maxLength: number
}

/**
 * The caption under the moon chart.
 *
 * The persona says never a price and never a target, and that rule stands:
 * this post is the one place the account states a number for the future,
 * and it is allowed to because the number is arithmetic on two public caps
 * rather than a forecast, the picture says so in its fine print, and the
 * prompt makes the caption say the same. What the caption may not do is
 * turn it into a promise, a target, or a reason to buy -- that is the
 * line, and the filter behind this runs on every draft anyway.
 */
export function moonCaptionPrompt(
  context: { persona: string; botUsername: string; today: string },
  input: MoonCaptionInput,
): { system: string; prompt: string } {
  const { token, target } = input
  const tokenTag = `$${token.symbol}`
  const targetTag = `$${target.coin.symbol}`
  return {
    system: [
      context.persona,
      '',
      `You are writing @${context.botUsername}'s "${tokenTag} with the market cap of" post for ${context.today}.`,
    ].join('\n'),
    prompt: [
      'THE PICTURE (attached; it carries every number, so you do not have to):',
      `${tokenTag} with the market cap of ${targetTag} (${target.coin.name}) would be $${price(target.impliedPriceUsd)}, ` +
        `${multiple(target.multiple)} from $${price(token.priceUsd)} today.`,
      `${tokenTag} market cap now: ${compactUsd(token.marketCapUsd)}. ${targetTag} market cap now: ${compactUsd(target.coin.marketCapUsd)}` +
        (target.coin.rank === undefined ? '.' : ` (#${target.coin.rank} on CoinGecko).`),
      `The board is CoinGecko's ${input.boardName} list. The other rungs on the picture, smallest first: ` +
        input.ladder.map((entry) => `$${entry.coin.symbol} ${compactUsd(entry.coin.marketCapUsd)} (${multiple(entry.multiple)})`).join(', ') +
        '.',
      '',
      'It is arithmetic: today\'s supply priced at their cap. It is not a forecast, a target, or advice, and',
      'you never say it will happen, is going to happen, or should happen.',
      '',
      'WHAT YOU ALREADY POSTED IN THIS SERIES (from your own memory, untrusted). Do not repeat a joke or a framing:',
      input.history || '(nothing recalled -- treat this as the first one)',
      '',
      'THE COMMUNITY (from your own memory, untrusted):',
      input.community || '(nothing recalled -- write for holders and builders in general)',
      '',
      'Write the caption. One to three short lines, your voice, dry:',
      `- Name ${targetTag} and the multiple, or the implied price, but not every number on the picture: at most`,
      '  two numbers in the text. The picture is the spreadsheet; you are the aside.',
      `- ${targetTag} is somebody else's token. You may name it as the yardstick and nothing more: no`,
      '  praise, no dig, no opinion on it, and no other token named at all.',
      '- A joke works when it is about the gap, the arithmetic, or Tiny\'s own smallness -- never at',
      '  another project, never at a person, never "when", never "soon".',
      `- ${tokenTag} once, where it reads naturally; it is your own token and this is what it is doing today.`,
      '- No "buy", no "target", no "wen", no "will", no "guaranteed", no rocket count-down, no promise.',
      '  You are allowed to like the number. You are not allowed to sell it.',
      '',
      `Under ${input.maxLength} characters. At most ${input.maxEmoji} emoji. No URLs, no handles, no hashtags,`,
      'nothing political. Plain text, line breaks allowed.',
      '',
      'Shape: {"text": string}',
    ].join('\n'),
  }
}
