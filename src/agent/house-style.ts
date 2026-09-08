/**
 * The account's hard formatting rules, applied to text that is about to be
 * published rather than only asked for in the prompt. SOUL.md states the emoji
 * cap as absolute and the humanizer skill bans em dashes; a model follows both
 * most of the time, which is not the same as always.
 */

/**
 * One emoji sequence: a pictographic base plus its variation selectors, skin-tone
 * modifiers, and any zero-width-joined parts, so a joined family counts once.
 */
const EMOJI_SEQUENCE =
  /\p{Extended_Pictographic}(️|\p{Emoji_Modifier})*(‍\p{Extended_Pictographic}(️|\p{Emoji_Modifier})*)*/gu

export function countEmoji(text: string): number {
  return text.match(EMOJI_SEQUENCE)?.length ?? 0
}

export interface HouseStyleResult {
  text: string
  /** What was changed, for logging. Empty when the draft was already clean. */
  fixes: string[]
}

/** Keeps the first `maxEmoji` sequences and drops the rest, tidying the gaps. */
function capEmoji(text: string, maxEmoji: number): string {
  let seen = 0
  return text
    .replace(EMOJI_SEQUENCE, (match) => {
      seen += 1
      return seen <= maxEmoji ? match : ''
    })
    .replace(/[ \t]{2,}/g, ' ')
    .replace(/[ \t]+([,.!?])/g, '$1')
    .trim()
}

export function applyHouseStyle(
  draft: string,
  options: { maxEmoji?: number; maxLength?: number; replyingTo?: string } = {},
): HouseStyleResult {
  const maxEmoji = options.maxEmoji ?? 1
  const maxLength = options.maxLength ?? 280
  const fixes: string[] = []
  let text = draft.trim()

  // X prefixes a reply with every participant's handle, not just the author
  // being answered, so any handle a draft opens with is published twice. The
  // observed failure was "@HappyTinyHuman @senamakel @senamakel ...", where the
  // draft opened with a participant that was not the one being replied to.
  const leadingHandles = /^(?:\s*@[A-Za-z0-9_]{1,15}\b[\s,:]*)+/
  if (leadingHandles.test(text)) {
    const stripped = text.replace(leadingHandles, '').trim()
    // A post that is nothing but handles has no reply left in it once they go.
    if (stripped) {
      text = stripped
      fixes.push('dropped leading @handles X adds itself')
    }
  }

  // The same handle further into the sentence is still a second tag, because
  // the prefix X adds is not part of the drafted text. Published, a mid-sentence
  // "@rafaelitewrld" reads back as "@rafaelitewrld @senamakel heyyy
  // @rafaelitewrld". The name is kept and only the @ goes, so the sentence
  // still addresses them.
  if (options.replyingTo) {
    const handle = options.replyingTo.replace(/^@/, '')
    const anywhere = new RegExp(`@(${handle})\\b`, 'gi')
    if (anywhere.test(text)) {
      text = text.replace(anywhere, '$1')
      fixes.push('unlinked a repeated @handle X already tags')
    }
  }

  // An em or en dash reads as machine-written, and the humanizer bans it. A
  // spaced dash becomes a comma; a tight one becomes a comma too, since it is
  // nearly always joining two clauses.
  if (/[—–]/.test(text)) {
    text = text.replace(/\s*[—–]\s*/g, ', ').replace(/,\s*,/g, ',')
    fixes.push('replaced em/en dashes')
  }

  if (/[“”‘’]/.test(text)) {
    text = text.replace(/[“”]/g, '"').replace(/[‘’]/g, "'")
    fixes.push('straightened curly quotes')
  }

  const emoji = countEmoji(text)
  if (emoji > maxEmoji) {
    text = capEmoji(text, maxEmoji)
    fixes.push(`cut ${emoji - maxEmoji} emoji over the cap of ${maxEmoji}`)
  }

  if (text.length > maxLength) {
    text = text.slice(0, maxLength).trimEnd()
    fixes.push(`trimmed to ${maxLength} characters`)
  }

  return { text, fixes }
}
