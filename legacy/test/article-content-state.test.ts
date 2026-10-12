import { describe, expect, it } from 'vitest'
import { markdownToContentState, parseInline } from '../src/article/content-state.js'

describe('markdown to the x article content state', () => {
  it('turns headings, paragraphs, lists and quotes into typed blocks', () => {
    const out = markdownToContentState(
      [
        '# the title is not a block',
        '',
        'the dek, on one line.',
        '',
        '## the kettle',
        '',
        'first line of a paragraph',
        'second line of the same paragraph.',
        '',
        '### a smaller heading',
        '',
        '- one thing',
        '- two things',
        '',
        '1. first',
        '2) second',
        '',
        '> the terminal said',
        '> something else',
        '',
        '---',
        '',
        'the end.',
      ].join('\n'),
      {},
    )
    expect(out.blocks.map((block) => [block.type, block.text])).toEqual([
      ['unstyled', 'the dek, on one line.'],
      ['header-two', 'the kettle'],
      ['unstyled', 'first line of a paragraph second line of the same paragraph.'],
      ['header-three', 'a smaller heading'],
      ['unordered-list-item', 'one thing'],
      ['unordered-list-item', 'two things'],
      ['ordered-list-item', 'first'],
      ['ordered-list-item', 'second'],
      ['blockquote', 'the terminal said\nsomething else'],
      ['atomic', ' '],
      ['unstyled', 'the end.'],
    ])
    expect(out.entities).toEqual([{ key: '0', value: { type: 'divider', mutability: 'immutable', data: {} } }])
    expect(out.blocks[9]!.entityRanges).toEqual([{ key: 0, offset: 0, length: 1 }])
  })

  it('places a picture as an atomic block pointing at an image entity, and drops one that was never drawn', () => {
    const out = markdownToContentState(
      ['## one', '', '![a kettle at night](image:s1)', '', 'text.', '', '## two', '', '![lost](image:s2)', '', 'more.'].join('\n'),
      { s1: { mediaId: 'm-1', caption: 'the kettle' } },
    )
    expect(out.blocks.map((block) => block.type)).toEqual(['header-two', 'atomic', 'unstyled', 'header-two', 'unstyled'])
    expect(out.blocks[1]!.entityRanges).toEqual([{ key: 0, offset: 0, length: 1 }])
    expect(out.entities).toEqual([
      {
        key: '0',
        value: {
          type: 'image',
          mutability: 'immutable',
          data: { mediaItems: [{ mediaCategory: 'tweet_image', mediaId: 'm-1' }], caption: 'the kettle' },
        },
      },
    ])
  })

  it('uses the placeholder caption when the image has none', () => {
    const out = markdownToContentState('![from the markdown](image:s1)', { s1: { mediaId: 'm-1' } })
    expect(out.entities[0]!.value.data.caption).toBe('from the markdown')
  })

  it('drops fenced code and never emits a markdown entity', () => {
    const out = markdownToContentState(['before', '', '```sh', 'rm -rf /', '```', '', 'after'].join('\n'), {})
    expect(out.blocks.map((block) => block.text)).toEqual(['before', 'after'])
    expect(out.entities).toEqual([])
  })

  it('handles the empty string', () => {
    expect(markdownToContentState('', {})).toEqual({ blocks: [], entities: [] })
  })
})

describe('inline styles', () => {
  it('strips the markers and records ranges over the stripped text', () => {
    const { text, ranges } = parseInline('a **bold** word, an _italic_ one, and ~~gone~~.')
    expect(text).toBe('a bold word, an italic one, and gone.')
    expect(ranges).toEqual([
      { offset: 2, length: 4, style: 'bold' },
      { offset: 16, length: 6, style: 'italic' },
      { offset: 32, length: 4, style: 'strikethrough' },
    ])
  })

  it('counts offsets in utf-16 code units, as x does', () => {
    const { text, ranges } = parseInline('🙂 **yes**')
    expect(text).toBe('🙂 yes')
    expect(ranges).toEqual([{ offset: 3, length: 3, style: 'bold' }])
  })

  it('leaves underscores inside words and lone asterisks alone', () => {
    expect(parseInline('the fridge_rate file and 2 * 3').text).toBe('the fridge_rate file and 2 * 3')
    expect(parseInline('the fridge_rate file and 2 * 3').ranges).toEqual([])
  })

  it('puts back a marker that never closed', () => {
    const { text, ranges } = parseInline('**not closed, just loud')
    expect(text).toBe('**not closed, just loud')
    expect(ranges).toEqual([])
  })

  it('nests bold inside italic', () => {
    const { text, ranges } = parseInline('_one **two** three_')
    expect(text).toBe('one two three')
    expect(ranges).toEqual([
      { offset: 0, length: 13, style: 'italic' },
      { offset: 4, length: 3, style: 'bold' },
    ])
  })
})
