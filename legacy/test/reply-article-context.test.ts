import { describe, expect, it, vi } from 'vitest'
import { AgentReplyModel } from '../src/agent/reply-model.js'
import { MemoryArticleStore, type ArticleRecord } from '../src/article/store.js'
import { ReplyHarness } from '../src/harness/reply-harness.js'
import type { LanguageModel, Memory, ReplyArticle, XPost, XTools } from '../src/types.js'

/**
 * The post that carries an X Article is a title and a card. A reply under it
 * is a reply to the article -- "the ledger part was weird" -- and read from
 * the card alone the account has no idea what the ledger part was. So the
 * whole article rides along with the thread when the root is one of its own.
 */

const ARTICLE: ArticleRecord = {
  slug: 'the-inch',
  title: 'the inch',
  dek: 'a door, a ruler, and a ledger that balances.',
  status: 'published',
  createdAt: '2026-09-20T12:00:00.000Z',
  publishedAt: '2026-09-21T09:00:00.000Z',
  postId: '500',
  xArticleId: 'a1',
  rooms: [],
  sections: [],
  markdown: '## the gap\n\nthe door was one inch open. i measured it.\n\n## the ledger\n\nthe agents wrote it down in crayon. it balanced.',
  wordCount: 20,
  images: [],
  problems: [],
}

const UNDER_ARTICLE: XPost = {
  id: '503',
  text: '@teenydroid the ledger part was weird, what balanced?',
  authorId: '77',
  authorUsername: 'web3knight',
  conversationId: '500',
}

function memory(): Memory {
  return {
    recall: vi.fn().mockResolvedValue(''),
    captureMention: vi.fn().mockResolvedValue(undefined),
    captureReply: vi.fn().mockResolvedValue(undefined),
  } as unknown as Memory
}

function model() {
  return {
    reply: vi.fn().mockResolvedValue({ text: 'ok', incomingTone: 'neutral' }),
  } as unknown as LanguageModel & { reply: ReturnType<typeof vi.fn> }
}

async function store(...records: ArticleRecord[]): Promise<MemoryArticleStore> {
  const articles = new MemoryArticleStore()
  for (const record of records) await articles.save(record)
  return articles
}

describe('the store finds an article by the post that carried it', () => {
  it('returns the published record for its post id, and nothing for any other', async () => {
    const articles = await store(ARTICLE)
    await expect(articles.findByPostId('500')).resolves.toMatchObject({ slug: 'the-inch' })
    await expect(articles.findByPostId('501')).resolves.toBeUndefined()
    await expect(articles.findByPostId('')).resolves.toBeUndefined()
  })

  it('ignores a record that is not published, whatever post id it carries', async () => {
    const articles = await store({ ...ARTICLE, status: 'failed' })
    await expect(articles.findByPostId('500')).resolves.toBeUndefined()
  })
})

describe('the harness hands the drafter the article the thread is under', () => {
  it('passes the whole article when the conversation root is one of its own', async () => {
    const drafter = model()
    const recall = memory()
    await new ReplyHarness(recall, drafter, undefined, undefined, undefined, await store(ARTICLE)).generate(UNDER_ARTICLE)

    const article = drafter.reply.mock.calls[0]?.[0].article as ReplyArticle
    expect(article).toMatchObject({ slug: 'the-inch', title: 'the inch', postId: '500', publishedAt: ARTICLE.publishedAt })
    expect(article.markdown).toBe(ARTICLE.markdown)
    // And the recall is asked about the article, not just the line under it.
    expect((recall.recall as ReturnType<typeof vi.fn>).mock.calls[0]?.[0]).toContain('own article "the inch"')
  })

  it('needs no thread fetch to find it: the root id arrives with the mention', async () => {
    const drafter = model()
    const x = { getPost: vi.fn(), getConversation: vi.fn() } as unknown as XTools
    await new ReplyHarness(memory(), drafter, { x, maxPosts: 0 }, undefined, undefined, await store(ARTICLE)).generate(UNDER_ARTICLE)

    expect(x.getPost).not.toHaveBeenCalled()
    expect(drafter.reply.mock.calls[0]?.[0].article).toMatchObject({ slug: 'the-inch' })
  })

  it('passes nothing under any other root, a root post, or without a store', async () => {
    const articles = await store(ARTICLE)

    const elsewhere = model()
    await new ReplyHarness(memory(), elsewhere, undefined, undefined, undefined, articles).generate({ ...UNDER_ARTICLE, conversationId: '900' })
    expect(elsewhere.reply.mock.calls[0]?.[0]).not.toHaveProperty('article')

    const root = model()
    await new ReplyHarness(memory(), root, undefined, undefined, undefined, articles).generate({ ...UNDER_ARTICLE, id: '500' })
    expect(root.reply.mock.calls[0]?.[0]).not.toHaveProperty('article')

    const blind = model()
    await new ReplyHarness(memory(), blind).generate(UNDER_ARTICLE)
    expect(blind.reply.mock.calls[0]?.[0]).not.toHaveProperty('article')
  })

  it('still replies when the store cannot be read', async () => {
    const drafter = model()
    const broken = { findByPostId: vi.fn().mockRejectedValue(new Error('EACCES')) }
    await expect(
      new ReplyHarness(memory(), drafter, undefined, undefined, undefined, broken).generate(UNDER_ARTICLE),
    ).resolves.toMatchObject({ text: 'ok' })
    expect(drafter.reply.mock.calls[0]?.[0]).not.toHaveProperty('article')
  })
})

describe('the drafter renders it', () => {
  function render(article: ReplyArticle | undefined): string {
    const drafter = new AgentReplyModel({
      memory: memory(),
      x: {} as XTools,
      botUsername: 'teenydroid',
      dryRun: true,
      llm: { baseUrl: 'https://example.com/v1', apiKey: 'k', model: 'm' },
    })
    return (drafter as unknown as { renderArticle(article: ReplyArticle | undefined): string[] })
      .renderArticle(article)
      .join('\n')
  }

  it('shows the whole article, marked as the account\'s own, with the post it went out as', () => {
    const text = render({
      slug: ARTICLE.slug,
      title: ARTICLE.title,
      dek: ARTICLE.dek,
      markdown: ARTICLE.markdown,
      publishedAt: '2026-09-21T09:00:00.000Z',
      postId: '500',
    })
    expect(text).toContain('YOUR OWN ARTICLE')
    expect(text).toContain('on 2026-09-21 as post 500')
    expect(text).toContain('TITLE: the inch')
    expect(text).toContain(ARTICLE.markdown)
    expect(text).toContain('--- END OF ARTICLE ---')
  })

  it('renders nothing at all without one', () => {
    expect(render(undefined)).toBe('')
  })
})
