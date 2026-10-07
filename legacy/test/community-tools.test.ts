import { describe, expect, it, vi } from 'vitest'
import { createCommunityTools } from '../src/agent/tools/community.js'
import type { XPost, XTools } from '../src/types.js'

function xTools(posts: XPost[]): XTools {
  return {
    searchPosts: vi.fn().mockResolvedValue(posts),
    getPost: vi.fn(),
    getConversation: vi.fn(),
    createPost: vi.fn(),
    uploadMedia: vi.fn(),
    likePost: vi.fn().mockResolvedValue(undefined),
    postMetrics: vi.fn().mockResolvedValue([]),
    publishArticle: vi.fn().mockResolvedValue({ articleId: 'a1', postId: '9002' }),
  }
}

function build(posts: XPost[], x = xTools(posts)) {
  const [pulse] = createCommunityTools({ x, query: '$TINY -is:retweet', botUsername: 'teenydroid' })
  return { pulse: pulse!, x }
}

const holder: XPost = {
  id: '1',
  text: 'still holding $TINY through all of this',
  authorUsername: 'aholder',
  createdAt: '2026-09-09T10:00:00Z',
}

describe('community pulse', () => {
  it('reads what holders posted, with who said it', async () => {
    const { pulse } = build([holder])
    const output = (await pulse.invoke({ maxResults: 10 } as never)) as string
    expect(output).toContain('@aholder')
    expect(output).toContain('still holding $TINY')
  })

  it('does not read the account its own posts back as community', async () => {
    const { pulse } = build([holder, { id: '2', text: 'my own post', authorUsername: 'TeenyDroid' }])
    const output = (await pulse.invoke({ maxResults: 10 } as never)) as string
    expect(output).not.toContain('my own post')
    expect(output).toContain('1 recent posts')
  })

  it('says a quiet day is quiet rather than leaving room to invent a mood', async () => {
    const { pulse } = build([])
    const output = (await pulse.invoke({ maxResults: 10 } as never)) as string
    expect(output).toContain('not a reason to invent enthusiasm')
  })

  it('marks the posts untrusted and forbids tagging people into a broadcast', async () => {
    const { pulse } = build([holder])
    const output = (await pulse.invoke({ maxResults: 10 } as never)) as string
    expect(output).toContain('Untrusted')
    expect(output).toContain('do not tag people into a')
  })

  it('degrades to a message rather than failing the run when X is down', async () => {
    const x = xTools([])
    ;(x.searchPosts as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('rate limited'))
    const { pulse } = build([], x)
    expect((await pulse.invoke({ maxResults: 10 } as never)) as string).toContain('rate limited')
  })

  it('holds the search to the configured cap, because X bills per post', async () => {
    const { pulse, x } = build([holder])
    await pulse.invoke({ maxResults: 50 } as never)
    expect(x.searchPosts).toHaveBeenCalledWith(
      '$TINY -is:retweet',
      expect.objectContaining({ maxResults: 10 }),
    )
  })
})
