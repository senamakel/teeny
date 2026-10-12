/** Forum posts arrive as HTML fragments with entities; the room reads plain text. */
export function stripHtml(html: string): string {
  return unescapeEntities(
    html
      .replace(/<br\s*\/?>/gi, '\n')
      .replace(/<\/p>/gi, '\n')
      .replace(/<[^>]+>/g, ''),
  )
    .replace(/[ \t]+/g, ' ')
    .replace(/\n{3,}/g, '\n\n')
    .trim()
}

const ENTITIES: Record<string, string> = {
  amp: '&',
  lt: '<',
  gt: '>',
  quot: '"',
  apos: "'",
  nbsp: ' ',
}

export function unescapeEntities(text: string): string {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (whole, code: string) => {
    if (code.startsWith('#x') || code.startsWith('#X')) return String.fromCodePoint(parseInt(code.slice(2), 16))
    if (code.startsWith('#')) return String.fromCodePoint(parseInt(code.slice(1), 10))
    return ENTITIES[code.toLowerCase()] ?? whole
  })
}

export function clip(text: string, maxChars: number): string {
  if (text.length <= maxChars) return text
  return `${text.slice(0, maxChars - 1).trimEnd()}…`
}

/** The words worth matching a forum post on: lowercase, no stop words, no shorties. */
export function keywords(query: string, max = 6): string[] {
  const stop = new Set(['to', 'be', 'or', 'of', 'in', 'on', 'at', 'is', 'it', 'as', 'an', 'by', 'up', 'so', 'if', 'no', 'we', 'he', 'me', 'my', 'do', 'us', 'the', 'and', 'for', 'with', 'that', 'this', 'from', 'into', 'are', 'was', 'were', 'has', 'have', 'its', 'about', 'what', 'when', 'where', 'who', 'how', 'why', 'out', 'not', 'but', 'you', 'your', 'our', 'they', 'them', 'there'])
  const seen = new Set<string>()
  const words: string[] = []
  for (const raw of query.toLowerCase().match(/[a-z0-9$][a-z0-9'-]*/g) ?? []) {
    const word = raw.replace(/^'+|'+$/g, '')
    if (word.length < 2 || stop.has(word) || seen.has(word)) continue
    seen.add(word)
    words.push(word)
    if (words.length >= max) break
  }
  return words
}
