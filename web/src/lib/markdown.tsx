import type { ReactNode } from 'react'

/**
 * The little markdown an article uses, rendered as elements: headings,
 * paragraphs, lists, quotes, bold, italic, strikethrough, dividers, and
 * `![caption](image:key)` where a picture goes. The same subset the bot
 * turns into X's blocks, so the site and the article on X read the same.
 * Nothing is injected as HTML.
 */

const IMAGE = /^!\[([^\]]*)\]\(image:([a-z0-9-]+)\)\s*$/

export interface RenderOptions {
  /** What to show where a picture goes; nothing, and the placeholder is dropped. */
  image?: (key: string, caption: string) => ReactNode
  /** Drop the leading italic line, the dek, when the page's header already shows it. */
  withoutDek?: boolean
}

/** The markdown without its first line when that is the italic dek the bot puts there. */
export function withoutDek(markdown: string): string {
  const lines = markdown.split('\n')
  const at = lines.findIndex((line) => line.trim())
  if (at < 0 || !/^_(.+)_$/.test(lines[at]!.trim())) return markdown
  return lines
    .slice(at + 1)
    .join('\n')
    .trim()
}

export function renderMarkdown(markdown: string, options: RenderOptions = {}): ReactNode[] {
  const out: ReactNode[] = []
  const lines = (options.withoutDek ? withoutDek(markdown) : markdown).replace(/\r\n/g, '\n').split('\n')
  let paragraph: string[] = []
  let list: { ordered: boolean; items: string[] } | undefined
  let quote: string[] = []
  let fenced = false
  let key = 0
  const next = (): number => key++

  const flush = (): void => {
    if (paragraph.length > 0) {
      out.push(<p key={next()}>{inline(paragraph.join(' '))}</p>)
      paragraph = []
    }
    if (list) {
      const items = list.items.map((item) => <li key={next()}>{inline(item)}</li>)
      out.push(list.ordered ? <ol key={next()}>{items}</ol> : <ul key={next()}>{items}</ul>)
      list = undefined
    }
    if (quote.length > 0) {
      out.push(<blockquote key={next()}>{inline(quote.join(' '))}</blockquote>)
      quote = []
    }
  }

  for (const raw of lines) {
    const line = raw.trimEnd()
    if (line.trim().startsWith('```')) {
      flush()
      fenced = !fenced
      continue
    }
    if (fenced) continue
    if (line.trim() === '') {
      flush()
      continue
    }
    const image = IMAGE.exec(line.trim())
    if (image) {
      flush()
      const node = options.image?.(image[2]!, image[1] ?? '')
      if (node) out.push(<figure key={next()}>{node}</figure>)
      continue
    }
    if (/^(-{3,}|\*{3,}|_{3,})$/.test(line.trim())) {
      flush()
      out.push(<hr key={next()} />)
      continue
    }
    const heading = /^(#{1,3})\s+(.*)$/.exec(line)
    if (heading) {
      flush()
      const text = inline(heading[2]!)
      const level = heading[1]!.length
      out.push(level === 1 ? <h1 key={next()}>{text}</h1> : level === 2 ? <h2 key={next()}>{text}</h2> : <h3 key={next()}>{text}</h3>)
      continue
    }
    const bullet = /^\s*[-*+]\s+(.*)$/.exec(line)
    const numbered = /^\s*\d+[.)]\s+(.*)$/.exec(line)
    if (bullet || numbered) {
      if (paragraph.length > 0 || quote.length > 0) flush()
      const ordered = Boolean(numbered)
      if (!list || list.ordered !== ordered) {
        if (list) flush()
        list = { ordered, items: [] }
      }
      list.items.push((bullet ?? numbered)![1]!)
      continue
    }
    const quoted = /^>\s?(.*)$/.exec(line)
    if (quoted) {
      if (paragraph.length > 0 || list) flush()
      quote.push(quoted[1]!)
      continue
    }
    if (list || quote.length > 0) flush()
    paragraph.push(line.trim())
  }
  flush()
  return out
}

/** `**bold**`, `_italic_`/`*italic*` and `~~strike~~`, one level deep. */
export function inline(text: string): ReactNode[] {
  const nodes: ReactNode[] = []
  const pattern = /(\*\*(.+?)\*\*|__(.+?)__|~~(.+?)~~|(?<![\w])_(?=\S)(.+?)(?<=\S)_(?![\w])|(?<![\w*])\*(?=\S)(.+?)(?<=\S)\*(?![\w*]))/g
  let last = 0
  let index = 0
  for (const match of text.matchAll(pattern)) {
    const at = match.index ?? 0
    if (at > last) nodes.push(text.slice(last, at))
    const [, , bold, bold2, strike, italic, italic2] = match
    if (bold !== undefined || bold2 !== undefined) nodes.push(<strong key={index++}>{bold ?? bold2}</strong>)
    else if (strike !== undefined) nodes.push(<s key={index++}>{strike}</s>)
    else nodes.push(<em key={index++}>{italic ?? italic2}</em>)
    last = at + match[0].length
  }
  if (last < text.length) nodes.push(text.slice(last))
  return nodes
}
