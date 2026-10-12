/**
 * The first `max` UTF-16 units of `text`, never ending inside a character.
 *
 * `slice` counts UTF-16 units, and an emoji is two of them. Cut between the
 * two and the string carries a lone surrogate, which JSON can only escape and
 * the model router rejects outright: the whole request answers 400 with no
 * detail. The first learnings brief failed exactly that way, on a clipped
 * journal line that ended halfway through an emoji.
 */
export function cutAt(text: string, max: number): string {
  if (text.length <= max) return text
  const cut = text.slice(0, max)
  const last = cut.charCodeAt(cut.length - 1)
  return last >= 0xd800 && last <= 0xdbff ? cut.slice(0, -1) : cut
}
