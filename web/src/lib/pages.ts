export type SceneName = 'brain' | 'stream' | 'soul' | 'lattice' | 'orbit' | 'grid' | 'shards'

export interface PageMeta {
  slug: string
  label: string
  eyebrow: string
  title: string
  lede: string
  scene: SceneName
}

export const PAGES: PageMeta[] = [
  {
    slug: '/reflections',
    label: 'Reflections',
    eyebrow: 'Reflections',
    title: 'What it thought before it spoke',
    lede: 'Every turn leaves a record: what came in, what memory said, which tools it called, the reasoning, what it said, and what it chose not to say. Handles are left out. The reasoning is not.',
    scene: 'stream',
  },
  {
    slug: '/soul',
    label: 'Soul',
    eyebrow: 'Soul',
    title: 'Who it is, in its own file',
    lede: 'The persona lives in SOUL.md, in the repo, where anyone can read the exact words the model is given. This is the short version.',
    scene: 'soul',
  },
  {
    slug: '/rules',
    label: 'Rules',
    eyebrow: 'Rules',
    title: 'What no message can change',
    lede: 'A rule written in a prompt is a request. A rule enforced by the tool set or a test is a fact. Each rule here says which it is.',
    scene: 'lattice',
  },
  {
    slug: '/tools',
    label: 'Tools',
    eyebrow: 'Tools',
    title: 'Everything it can do. And what it cannot.',
    lede: 'This is the whole list. A test asserts it exactly, so adding a capability is a deliberate act that fails the suite until someone decides.',
    scene: 'orbit',
  },
  {
    slug: '/memory',
    label: 'Memory',
    eyebrow: 'Memory',
    title: 'What it remembers, and what it refuses to',
    lede: 'Long-term memory is CortexDB. The order things happen in matters as much as what is stored.',
    scene: 'grid',
  },
  {
    slug: '/lessons',
    label: 'Lessons',
    eyebrow: 'Lessons',
    title: 'Things it got wrong, on the record',
    lede: 'Each of these is a mistake made in public and a change made in the code because of it. The comments in the repo tell the same story.',
    scene: 'shards',
  },
]

export function pageBySlug(slug: string): PageMeta {
  const page = PAGES.find((p) => p.slug === slug)
  if (!page) throw new Error(`no page registered for ${slug}`)
  return page
}

export function neighbours(slug: string): { prev?: PageMeta; next?: PageMeta } {
  const i = PAGES.findIndex((p) => p.slug === slug)
  return { prev: PAGES[i - 1], next: PAGES[i + 1] }
}
