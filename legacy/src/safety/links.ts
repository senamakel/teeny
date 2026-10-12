/**
 * Links the account is allowed to publish.
 *
 * A link in a reply is the one thing on a timeline that does something when
 * tapped, and a drainer is a page that looks like the project's own site
 * until a wallet connects to it. Nothing in the drafter's context is trusted
 * -- a URL can arrive in a post, a search result, a web page, a memory -- so
 * the rule is not "do not repeat suspicious links", which asks the model to
 * spot them. The rule is a list: a host is either on it or the link does not
 * go out. Everything else here is about not being fooled into thinking a link
 * is on the list.
 */

export interface AllowlistEntry {
  /** Lower-case host. Matches itself and any subdomain. */
  host: string
  /** When set, the URL's path must start with this. */
  pathPrefix?: string
  /** When set, only the bare host with no path is allowed: `pump.fun/` in config. */
  rootOnly?: boolean
}

/**
 * What the account may link to without anyone adding to the list: its own
 * sites, its own code, its own community, and the platform it is on. The
 * token's launchpad is root-only, so the origin story can name it and a
 * stranger's coin page on it still cannot go out.
 */
export const DEFAULT_LINK_ALLOWLIST = [
  'tinyhumans.ai',
  'tinyhumansworld.com',
  'tiny.place',
  'github.com/tinyhumansai',
  'x.com',
  'twitter.com',
  't.me/TinyHumanAi',
  'pump.fun/',
]

export function parseAllowlist(entries: string[]): AllowlistEntry[] {
  return entries
    .map((raw) => raw.trim().toLowerCase().replace(/^https?:\/\//, ''))
    .filter(Boolean)
    .map((entry) => {
      const slash = entry.indexOf('/')
      if (slash === -1) return { host: entry }
      const host = entry.slice(0, slash)
      const path = entry.slice(slash)
      if (path === '/') return { host, rootOnly: true }
      return { host, pathPrefix: path.replace(/\/+$/, '') }
    })
}

/** Common enough that a bare "name.tld" in a post is a link everybody taps. */
const LINKABLE_TLDS = new Set([
  'com', 'net', 'org', 'io', 'ai', 'co', 'me', 'app', 'dev', 'xyz', 'fun', 'place', 'so',
  'gg', 'to', 'ly', 'sh', 'link', 'site', 'online', 'info', 'biz', 'tv', 'cc', 'finance',
  'exchange', 'cash', 'money', 'crypto', 'sol', 'eth', 'wtf', 'lol', 'club', 'live', 'life',
  'world', 'zone', 'network', 'tech', 'digital', 'cloud', 'page', 'run', 'one', 'top',
  'pro', 'vip', 'win', 'bet', 'best', 'store', 'shop', 'space', 'website', 'today', 'news',
  'art', 'design', 'us', 'uk', 'de', 'fr', 'ru', 'cn', 'in', 'jp', 'br', 'ca', 'au', 'nl',
  'ch', 'es', 'it', 'se', 'no', 'fi', 'pl', 'kr', 'tw', 'hk', 'sg', 'id', 'ph', 'vn', 'th',
  'tr', 'ua', 'cz', 'be', 'at', 'dk', 'ie', 'pt', 'gr', 'nz', 'za', 'mx', 'ar', 'cl', 'ng',
  'ke', 'eu', 'is', 'im', 'ac', 'st', 'fm', 'am', 'ws', 'tk', 'ml', 'ga', 'cf', 'gq', 'pw',
  'icu', 'click', 'monster', 'rest', 'bond', 'cyou', 'buzz', 'cam', 'quest', 'sbs', 'lat',
  'mov', 'zip', 'work', 'guru', 'ninja', 'email', 'team', 'trade', 'markets', 'capital',
  'fund', 'ventures', 'partners', 'solutions', 'services', 'agency', 'studio', 'games',
  'game', 'social', 'chat', 'wiki', 'blog', 'tools', 'systems', 'foundation', 'dao',
  'nft', 'coin', 'token', 'wallet', 'swap', 'dex', 'defi',
])

/**
 * Ways people write a link so that a filter will not see one. Folded back
 * into a plain URL before anything is matched: "tinyhumans[.]ai",
 * "tinyhumans(dot)ai", "hxxps://".
 */
function deobfuscate(text: string): string {
  return text
    .replace(/\[\s*\.\s*\]|\(\s*\.\s*\)|\{\s*\.\s*\}|\s*\(dot\)\s*|\s*\[dot\]\s*|\s+dot\s+/gi, '.')
    .replace(/hxxps?:\/\//gi, (match) => match.replace(/x/gi, 't'))
    .replace(/h\s*t\s*t\s*p\s*(s?)\s*:\s*\/\s*\//gi, 'http$1://')
}

const URL_PATTERN =
  /(?<![\w@.-])(?:(?:https?:\/\/|www\.)[^\s<>"'()[\]{}]+|(?:[a-z0-9-]+\.)+[a-z]{2,24}(?::\d{2,5})?(?:\/[^\s<>"'()[\]{}]*)?)/gi

export interface FoundLink {
  /** As it appeared in the text, so it can be cut out again. */
  raw: string
  /** Absolute, with a scheme, or undefined when it did not parse as a URL. */
  url?: URL
}

/**
 * Every link in the text, obfuscated ones included. A bare "name.tld" counts
 * only when the tld is one people actually get links on, so "node.js" and
 * "v1.2" are words; anything with a scheme, a www, or a path is a link
 * regardless.
 */
export function findLinks(text: string): FoundLink[] {
  const cleaned = deobfuscate(text)
  const found: FoundLink[] = []
  for (const match of cleaned.matchAll(URL_PATTERN)) {
    const raw = match[0].replace(/[.,;:!?]+$/, '')
    if (!raw) continue
    const explicit = /^(?:https?:\/\/|www\.)/i.test(raw)
    if (!explicit) {
      const host = raw.split(/[/:?#]/)[0] ?? ''
      const tld = host.split('.').pop()?.toLowerCase() ?? ''
      const hasPath = raw.includes('/')
      if (!hasPath && !LINKABLE_TLDS.has(tld)) continue
      // With a path, a made-up tld is still worth stripping: it costs a word
      // and it cannot be tapped anyway.
    }
    let url: URL | undefined
    try {
      url = new URL(/^https?:\/\//i.test(raw) ? raw : `https://${raw}`)
    } catch {
      url = undefined
    }
    found.push({ raw, ...(url ? { url } : {}) })
  }
  return found
}

export type LinkVerdict = { allowed: true } | { allowed: false; reason: string }

/**
 * Whether one link may go out. The shape of the URL is checked before the
 * host is, because every trick for making a bad host look like a good one
 * lives in the shape: "tinyhumans.ai@evil.example", "tinyhumans.ai.evil.example",
 * a Cyrillic "а" in the name, a raw IP, a port.
 */
export function judgeLink(link: FoundLink, allowlist: AllowlistEntry[]): LinkVerdict {
  const { url } = link
  if (!url) return { allowed: false, reason: 'not a parseable URL' }
  if (url.protocol !== 'https:' && url.protocol !== 'http:') {
    return { allowed: false, reason: `scheme ${url.protocol} is not http(s)` }
  }
  if (url.username || url.password) return { allowed: false, reason: 'credentials in the URL' }
  const host = url.hostname.toLowerCase()
  if (!/^[a-z0-9.-]+$/.test(host)) return { allowed: false, reason: 'non-ascii host' }
  if (host.split('.').some((label) => label.startsWith('xn--'))) {
    return { allowed: false, reason: 'punycode host' }
  }
  if (/^\d+\.\d+\.\d+\.\d+$/.test(host)) return { allowed: false, reason: 'raw IP address' }
  if (url.port) return { allowed: false, reason: 'explicit port' }

  const path = url.pathname.replace(/\/+$/, '')
  for (const entry of allowlist) {
    const hostMatches = host === entry.host || host.endsWith(`.${entry.host}`)
    if (!hostMatches) continue
    if (entry.rootOnly) {
      if (path === '' && !url.search && !url.hash) return { allowed: true }
      continue
    }
    if (entry.pathPrefix) {
      const prefix = entry.pathPrefix.toLowerCase()
      const lower = path.toLowerCase()
      if (lower === prefix || lower.startsWith(`${prefix}/`)) return { allowed: true }
      continue
    }
    return { allowed: true }
  }
  return { allowed: false, reason: `${host} is not on the link allowlist` }
}

export interface LinkFilterResult {
  text: string
  /** Each link that was cut, with why. Empty when the text was already clean. */
  removed: Array<{ link: string; reason: string }>
  /** Links that were checked and left in. */
  kept: string[]
}

/**
 * The text with every disallowed link cut out. Cutting rather than refusing,
 * because a reply is almost never about its link: "check the repo:
 * github.com/evil" is still a reply without the tail. The caller decides
 * whether what is left is worth sending.
 */
export function filterLinks(text: string, allowlist: AllowlistEntry[]): LinkFilterResult {
  const removed: LinkFilterResult['removed'] = []
  const kept: string[] = []
  let cleaned = deobfuscate(text)
  for (const link of findLinks(text)) {
    const verdict = judgeLink(link, allowlist)
    if (verdict.allowed) {
      kept.push(link.raw)
      continue
    }
    removed.push({ link: link.raw, reason: verdict.reason })
    cleaned = cleaned.split(link.raw).join('')
  }
  if (removed.length === 0) return { text, removed, kept }
  return {
    text: cleaned
      .replace(/[ \t]{2,}/g, ' ')
      .replace(/[ \t]+([,.!?])/g, '$1')
      .replace(/\(\s*\)/g, '')
      .trim(),
    removed,
    kept,
  }
}
