import { isPolitical } from '../morning/news.js'
import { screenText } from '../safety/screen.js'
import type { ForumPost } from './types.js'

/**
 * What scraped text has to pass before it is allowed into an agent's
 * context or a tool result. The narrative direction of the lexicon lets
 * swearing through and keeps every hard category; the political filter is
 * the morning post's. 4chan is where the slurs are, and a slur that gets
 * into the room gets said back.
 */
export function admissible(text: string): boolean {
  if (!text.trim()) return false
  if (screenText(text, 'narrative')) return false
  if (SENSITIVE.test(text)) return false
  return !isPolitical({ title: '', excerpt: text })
}

/**
 * Groups of people the room has no business riffing on. The lexicon catches
 * slurs; this catches the thread that is about a religion or an ethnicity
 * without using one, which on the boards is most of them. A robot escaping
 * a lab does not need any of it.
 */
const SENSITIVE = new RegExp(
  '(?:^|[^a-z])(?:' +
    [
      'jew', 'jews', 'jewish', 'zionist', 'zionism', 'israel', 'israeli', 'palestin\\w*', 'gaza', 'holocaust',
      'muslim', 'muslims', 'islam', 'islamic', 'islamist', 'quran', 'koran', 'sharia', 'jihad\\w*',
      'christian', 'christians', 'christianity', 'catholic', 'protestant', 'evangelical', 'hindu', 'hindus', 'hinduism',
      'sikh', 'buddhist', 'atheist', 'bible', 'torah', 'talmud', 'satan\\w*', 'antichrist',
      'race', 'racial', 'racist', 'racism', 'ethnic\\w*', 'white people', 'black people', 'whites', 'blacks',
      'immigrant\\w*', 'migrant\\w*', 'refugee\\w*', 'genocide', 'ethnostate', 'great replacement',
      'trans\\b', 'transgender', 'gay', 'lgbt\\w*', 'homosexual\\w*',
    ].join('|') +
    ')(?:$|[^a-z])',
  'i',
)

export function admissiblePost(post: ForumPost): boolean {
  return admissible(`${post.title ?? ''}\n${post.text}`)
}

export function renderPost(post: ForumPost): string {
  const score =
    post.score === undefined ? '' : post.site === 'reddit' ? ` ↑${post.score}` : ` ${post.score} replies`
  const head = `[${post.where}${score}]`
  const title = post.title && post.title !== post.text ? `${post.title} — ` : ''
  return `${head} ${title}${post.text.replace(/\s*\n+\s*/g, ' / ')}`
}
