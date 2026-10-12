import { unescapeEntities } from './text.js'

export interface RssItem {
  title: string
  source?: string
  at?: string
  description?: string
}

/**
 * Google News as a feed: no key, no account, a title and a source per item.
 * It is the fallback seed when Exa is over capacity, which it is often
 * enough that a rehearsal cannot depend on it. The feed's own terms limit
 * it to reading; nothing here is stored or republished.
 */
export async function googleNews(query: string, fetchImpl: typeof fetch): Promise<RssItem[]> {
  const url = `https://news.google.com/rss/search?q=${encodeURIComponent(query)}&hl=en-US&gl=US&ceid=US:en`
  const response = await fetchImpl(url, { headers: { 'user-agent': 'teeny-backrooms/1.0' } })
  if (!response.ok) throw new Error(`google news answered ${response.status}`)
  return parseRss(await response.text())
}

export function parseRss(xml: string): RssItem[] {
  const items: RssItem[] = []
  for (const [, body] of xml.matchAll(/<item>([\s\S]*?)<\/item>/g)) {
    const field = (name: string): string | undefined => {
      const match = new RegExp(`<${name}[^>]*>(?:<!\\[CDATA\\[)?([\\s\\S]*?)(?:\\]\\]>)?<\\/${name}>`).exec(body ?? '')
      // Descriptions arrive as escaped html inside the xml: unescape, strip, unescape again.
      return match?.[1] ? unescapeEntities(unescapeEntities(match[1]).replace(/<[^>]+>/g, '')).replace(/\s+/g, ' ').trim() : undefined
    }
    const rawTitle = field('title')
    if (!rawTitle) continue
    const source = field('source')
    // Google appends " - Publisher" to every title; the source element already has it.
    const title = source && rawTitle.endsWith(` - ${source}`) ? rawTitle.slice(0, -(source.length + 3)) : rawTitle
    const at = field('pubDate')
    const description = field('description')
    items.push({
      title,
      ...(source ? { source } : {}),
      ...(at ? { at } : {}),
      ...(description ? { description } : {}),
    })
  }
  return items
}
