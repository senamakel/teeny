import type { XArticleBlock, XArticleContentState, XArticleEntity, XArticleInlineStyle } from '../types.js'

/**
 * Markdown to the shape the X Articles API takes.
 *
 * The article is written as markdown because that is what a model writes
 * well and what the site renders; X wants a list of typed blocks with the
 * inline styles as offset ranges and every picture as an atomic block that
 * points at an entity. This is the one place that knows both. It handles
 * the subset the pipeline emits: headings, paragraphs, lists, quotes, bold,
 * italic, strikethrough, dividers and image placeholders. Everything else
 * degrades to plain text rather than failing, so a stray fence or table
 * costs a little formatting, never the article.
 *
 * No `markdown` entity is ever emitted: those carry a 10,000-character cap
 * on X's side and native blocks do not.
 */

/** `![caption](image:s1)`: where a section's picture goes. */
export const IMAGE_PLACEHOLDER = /^!\[([^\]]*)\]\(image:([a-z0-9-]+)\)\s*$/

export interface ArticleImageRef {
  mediaId: string
  caption?: string
}

export function markdownToContentState(markdown: string, images: Record<string, ArticleImageRef>): XArticleContentState {
  const blocks: XArticleBlock[] = []
  const entities: XArticleEntity[] = []
  const lines = markdown.replace(/\r\n/g, '\n').split('\n')

  let paragraph: string[] = []
  const flush = (): void => {
    if (paragraph.length === 0) return
    blocks.push(textBlock('unstyled', paragraph.join(' ')))
    paragraph = []
  }
  const atomic = (entity: XArticleEntity['value']): void => {
    const key = entities.length
    entities.push({ key: String(key), value: entity })
    blocks.push({ text: ' ', type: 'atomic', entityRanges: [{ key, offset: 0, length: 1 }] })
  }

  let fenced = false
  // Consecutive `> ` lines are one quote; the source is kept so the styles
  // are parsed over the whole of it.
  let quote: string | undefined
  const closeQuote = (): void => {
    if (quote === undefined) return
    blocks.push(textBlock('blockquote', quote))
    quote = undefined
  }
  for (const raw of lines) {
    const line = raw.trimEnd()
    if (line.trim().startsWith('```')) {
      // Code has no block type here; the fence and its contents are dropped.
      flush()
      closeQuote()
      fenced = !fenced
      continue
    }
    if (fenced) continue
    if (line.trim() === '') {
      flush()
      closeQuote()
      continue
    }
    const image = IMAGE_PLACEHOLDER.exec(line.trim())
    if (image) {
      flush()
      closeQuote()
      const ref = images[image[2]!]
      if (ref) {
        atomic({
          type: 'image',
          mutability: 'immutable',
          data: {
            mediaItems: [{ mediaCategory: 'tweet_image', mediaId: ref.mediaId }],
            ...(ref.caption ?? image[1] ? { caption: ref.caption ?? image[1] } : {}),
          },
        })
      }
      continue
    }
    if (/^(-{3,}|\*{3,}|_{3,})$/.test(line.trim())) {
      flush()
      closeQuote()
      atomic({ type: 'divider', mutability: 'immutable', data: {} })
      continue
    }
    const heading = /^(#{1,3})\s+(.*)$/.exec(line)
    if (heading) {
      flush()
      closeQuote()
      // The title travels separately; an H1 in the body would repeat it.
      if (heading[1]!.length === 1) continue
      blocks.push(textBlock(heading[1]!.length === 2 ? 'header-two' : 'header-three', heading[2]!))
      continue
    }
    const bullet = /^\s*[-*+]\s+(.*)$/.exec(line)
    if (bullet) {
      flush()
      closeQuote()
      blocks.push(textBlock('unordered-list-item', bullet[1]!))
      continue
    }
    const numbered = /^\s*\d+[.)]\s+(.*)$/.exec(line)
    if (numbered) {
      flush()
      closeQuote()
      blocks.push(textBlock('ordered-list-item', numbered[1]!))
      continue
    }
    const quoted = /^>\s?(.*)$/.exec(line)
    if (quoted) {
      flush()
      quote = quote === undefined ? quoted[1]! : `${quote}\n${quoted[1]!}`
      continue
    }
    closeQuote()
    paragraph.push(line.trim())
  }
  flush()
  closeQuote()
  return { blocks, entities }
}

function textBlock(type: XArticleBlock['type'], source: string): XArticleBlock {
  const { text, ranges } = parseInline(source)
  return { text, type, ...(ranges.length > 0 ? { inlineStyleRanges: ranges } : {}) }
}

/**
 * Inline markers to style ranges. Offsets are in UTF-16 code units of the
 * text with the markers removed, which is how X (and JavaScript) count.
 */
export function parseInline(source: string): { text: string; ranges: XArticleInlineStyle[] } {
  const ranges: XArticleInlineStyle[] = []
  let text = ''
  let index = 0
  const open: Array<{ style: XArticleInlineStyle['style']; marker: string; at: number }> = []
  while (index < source.length) {
    const rest = source.slice(index)
    const marker = /^(\*\*|__|~~|\*|_)/.exec(rest)?.[1]
    if (marker && isMarkerHere(source, index, marker, open)) {
      const style: XArticleInlineStyle['style'] = marker === '~~' ? 'strikethrough' : marker.length === 2 ? 'bold' : 'italic'
      const opened = open.findIndex((item) => item.marker === marker)
      if (opened >= 0) {
        const { at } = open.splice(opened, 1)[0]!
        if (text.length > at) ranges.push({ offset: at, length: text.length - at, style })
      } else {
        open.push({ style, marker, at: text.length })
      }
      index += marker.length
      continue
    }
    text += source[index]
    index += 1
  }
  // A marker never closed was punctuation, not a style: put it back.
  for (const item of open.reverse()) {
    text = `${text.slice(0, item.at)}${item.marker}${text.slice(item.at)}`
    for (const range of ranges) if (range.offset >= item.at) range.offset += item.marker.length
  }
  ranges.sort((a, b) => a.offset - b.offset)
  return { text, ranges }
}

/**
 * A `_` inside a word (snake_case) or a `*` between spaces is not a marker.
 * An opening marker needs a non-space after it; a closing one a non-space
 * before it.
 */
function isMarkerHere(
  source: string,
  index: number,
  marker: string,
  open: Array<{ marker: string }>,
): boolean {
  const before = source[index - 1] ?? ' '
  const after = source[index + marker.length] ?? ' '
  const closing = open.some((item) => item.marker === marker)
  if (marker === '_' || marker === '__') {
    if (/\w/.test(before) && /\w/.test(after)) return false
  }
  return closing ? !/\s/.test(before) : !/\s/.test(after)
}
