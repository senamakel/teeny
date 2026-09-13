import { describe, expect, it, vi } from 'vitest'
import { createExaTools, type ExaSearch } from '../src/agent/tools/exa.js'
import { createMemoryTools } from '../src/agent/tools/memory.js'
import { createXTools } from '../src/agent/tools/x.js'
import { createMediaTools } from '../src/agent/tools/media.js'
import { MediaBudget, MediaStore } from '../src/media/store.js'
import { lexicalGate } from '../src/safety/gate.js'
import type { Memory, XPost, XTools } from '../src/types.js'

function toolsByName(tools: Array<{ name: string; invoke: (input: never) => Promise<unknown> }>) {
  return new Map(tools.map((tool) => [tool.name, tool]))
}

function xTools(): XTools {
  const post: XPost = {
    id: '10',
    text: 'hello @stonksrocks',
    authorUsername: 'someone',
    conversationId: '9',
  }
  return {
    searchPosts: vi.fn().mockResolvedValue([post]),
    getPost: vi.fn().mockResolvedValue(post),
    getConversation: vi.fn().mockResolvedValue([post]),
    createPost: vi.fn().mockResolvedValue({ id: '77' }),
    uploadMedia: vi.fn().mockResolvedValue({ mediaId: 'media-1' }),
  }
}

describe('X tools', () => {
  it('renders search results with the ids the agent needs to follow up', async () => {
    const x = xTools()
    const tools = toolsByName(createXTools(x, { dryRun: true }))
    const output = await tools.get('x_search_posts')!.invoke({ query: '@stonksrocks' } as never)

    expect(output).toContain('id: 10')
    expect(output).toContain('conversation_id: 9')
    expect(output).toContain('author: @someone')
  })

  it('does not publish in dry-run mode', async () => {
    const x = xTools()
    const tools = toolsByName(createXTools(x, { dryRun: true }))
    const output = await tools.get('x_post')!.invoke({ text: 'gm' } as never)

    expect(output).toContain('DRY RUN')
    expect(x.createPost).not.toHaveBeenCalled()
  })

  it('publishes replies when dry-run is off', async () => {
    const x = xTools()
    const tools = toolsByName(createXTools(x, { dryRun: false }))
    const output = await tools
      .get('x_post')!
      .invoke({ text: 'gm', replyToPostId: '10' } as never)

    expect(x.createPost).toHaveBeenCalledWith('gm', { replyToPostId: '10' })
    expect(output).toBe('Published post 77.')
  })

  it('lists a post\'s pictures so the agent can look at them', async () => {
    const x = xTools()
    ;(x.getPost as ReturnType<typeof vi.fn>).mockResolvedValue({
      id: '10',
      text: 'look',
      media: [
        { type: 'photo', url: 'https://pbs.twimg.com/media/a.jpg', altText: 'a chart' },
        { type: 'video', previewUrl: 'https://pbs.twimg.com/media/b.jpg' },
      ],
    })
    const tools = toolsByName(createXTools(x, { dryRun: true }))
    const output = await tools.get('x_get_post')!.invoke({ id: '10' } as never)

    expect(output).toContain('media: photo https://pbs.twimg.com/media/a.jpg (alt: a chart)')
    expect(output).toContain('media: video https://pbs.twimg.com/media/b.jpg')
  })

  it('attaches a generated file by its id, and refuses an id it does not hold', async () => {
    const x = xTools()
    const media = new MediaStore()
    const stored = media.put({ kind: 'image', prompt: 'p', data: new Uint8Array([1]), mimeType: 'image/png' })
    const tools = toolsByName(createXTools(x, { dryRun: false, media }))

    const output = await tools.get('x_post')!.invoke({ text: 'gm', mediaId: stored.id } as never)
    expect(x.uploadMedia).toHaveBeenCalledWith({ data: new Uint8Array([1]), mimeType: 'image/png' })
    expect(x.createPost).toHaveBeenCalledWith('gm', { mediaIds: ['media-1'] })
    expect(output).toBe('Published post 77 with a generated image.')

    const refused = await tools.get('x_post')!.invoke({ text: 'gm', mediaId: 'img_madeup_1' } as never)
    expect(refused).toContain('Refused')
    expect(x.createPost).toHaveBeenCalledTimes(1)
  })

  it('omits the posting tool from a research-only tool set', () => {
    const names = createXTools(xTools(), { dryRun: false, allowPosting: false }).map(
      (tool) => tool.name,
    )
    expect(names).not.toContain('x_post')
  })
})

describe('memory tools', () => {
  it('recalls context and stores durable notes', async () => {
    const memory: Memory = {
      captureActivity: vi.fn().mockResolvedValue(undefined),
      captureMention: vi.fn().mockResolvedValue(undefined),
      recall: vi.fn().mockResolvedValue('stonks.rocks is an open-source dapp'),
      captureReply: vi.fn().mockResolvedValue(undefined),
    }
    const tools = toolsByName(createMemoryTools(memory))

    await expect(tools.get('recall_memory')!.invoke({ query: 'what is it' } as never)).resolves.toBe(
      'stonks.rocks is an open-source dapp',
    )
    await tools.get('remember')!.invoke({ note: 'launched v3' } as never)
    expect(memory.captureActivity).toHaveBeenCalledWith(
      'agent_note',
      expect.stringMatching(/^note:/),
      { note: 'launched v3', labels: [] },
    )
  })
})

describe('Exa tools', () => {
  it('formats search hits with urls and excerpts', async () => {
    const exa = {
      searchAndContents: vi.fn().mockResolvedValue({
        results: [
          {
            title: 'Launch notes',
            url: 'https://example.com/a',
            publishedDate: '2026-01-02',
            text: 'the dapp shipped',
          },
        ],
      }),
      getContents: vi.fn(),
      findSimilarAndContents: vi.fn(),
      answer: vi.fn().mockResolvedValue({
        answer: '42',
        citations: [{ url: 'https://example.com/b', title: 'B' }],
      }),
    } as unknown as ExaSearch
    const tools = toolsByName(createExaTools(exa))

    const search = await tools.get('web_search')!.invoke({ query: 'stonks.rocks' } as never)
    expect(search).toContain('https://example.com/a')
    expect(search).toContain('excerpt: the dapp shipped')

    const answer = await tools.get('web_answer')!.invoke({ question: 'what' } as never)
    expect(answer).toContain('42')
    expect(answer).toContain('https://example.com/b')
  })
})

describe('buildTools', () => {
  it('includes Exa tools only when an Exa key is configured', async () => {
    const { buildTools } = await import('../src/agent/graph.js')
    const options = {
      memory: {
        captureActivity: vi.fn(),
        captureMention: vi.fn(),
        recall: vi.fn(),
        captureReply: vi.fn(),
      } as unknown as Memory,
      x: xTools(),
      botUsername: 'stonksrocks',
      dryRun: true,
      llm: { baseUrl: 'https://example.com/v1', apiKey: 'k', model: 'm' },
    }

    expect(buildTools(options).map((tool) => tool.name)).toEqual([
      'recall_memory',
      'remember',
      'x_search_posts',
      'x_get_post',
      'x_get_comments',
      'x_post',
    ])
    expect(buildTools({ ...options, exaApiKey: 'exa-key' }).map((tool) => tool.name)).toContain(
      'web_search',
    )
  })
})

describe('emoji limit', () => {
  it('counts an emoji sequence once', async () => {
    const { countEmoji } = await import('../src/agent/house-style.js')
    expect(countEmoji('gm 🛠️')).toBe(1)
    expect(countEmoji('gm 🛠️ 🚀')).toBe(2)
    expect(countEmoji('family 👨‍👩‍👧 here')).toBe(1)
    expect(countEmoji('wave 👋🏽 once')).toBe(1)
    expect(countEmoji('plain text')).toBe(0)
  })

  it('refuses a post with more emoji than the account allows', async () => {
    const x = xTools()
    const tools = toolsByName(createXTools(x, { dryRun: false }))
    const output = await tools.get('x_post')!.invoke({ text: 'gm frens 🛠️ 🚀' } as never)

    expect(output).toContain('Refused')
    expect(x.createPost).not.toHaveBeenCalled()
  })
})

describe('content filter on the publishing tools', () => {
  it('x_post refuses a draft the filter holds, and tells the model not to rephrase', async () => {
    const x = xTools()
    const tools = toolsByName(createXTools(x, { dryRun: false, safety: lexicalGate() }))
    const output = await tools.get('x_post')!.invoke({ text: 'this is fucking great' } as never)

    expect(output).toContain('Refused')
    expect(output).toContain('profanity')
    expect(x.createPost).not.toHaveBeenCalled()
  })

  it('x_post refuses a link off the allowlist rather than publishing a gap', async () => {
    const x = xTools()
    const tools = toolsByName(createXTools(x, { dryRun: false, safety: lexicalGate() }))
    const output = await tools.get('x_post')!.invoke({ text: 'claim at https://claim-tiny.xyz/go' } as never)

    expect(output).toContain('not on the allowlist')
    expect(x.createPost).not.toHaveBeenCalled()
  })

  it('x_post publishes a clean draft with an allowlisted link', async () => {
    const x = xTools()
    const tools = toolsByName(createXTools(x, { dryRun: false, safety: lexicalGate() }))
    await tools.get('x_post')!.invoke({ text: 'code at github.com/tinyhumansai' } as never)

    expect(x.createPost).toHaveBeenCalledWith('code at github.com/tinyhumansai', {})
  })

  it('generate_image refuses a prohibited prompt before spending the budget', async () => {
    const images = { generate: vi.fn().mockResolvedValue({ data: new Uint8Array([1]), mimeType: 'image/png' }) }
    const budget = new MediaBudget({ imagesPerDay: 1, videosPerDay: 0 })
    const tools = toolsByName(
      createMediaTools({ store: new MediaStore(), budget, images }, { safety: lexicalGate() }),
    )
    const output = await tools.get('generate_image')!.invoke({ prompt: 'Tiny naked on a beach' } as never)

    expect(output).toContain('Refused')
    expect(images.generate).not.toHaveBeenCalled()
    expect(budget.remaining('image')).toBe(1)
  })
})
