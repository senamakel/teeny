import { describe, expect, it } from 'vitest'
import { loadCalendar, parseCalendar } from '../src/calendar/calendar.js'

const gm = {
  name: 'gm',
  cron: '0 7 * * *',
  channels: ['x', 'telegram'],
  media: 'image',
  brief: 'A good-morning post, one line, Tiny with the morning.',
}

describe('the content calendar', () => {
  it('parses an item and fills in the defaults', () => {
    const calendar = parseCalendar(JSON.stringify({ items: [gm] }))
    expect(calendar.items[0]).toMatchObject({
      name: 'gm',
      media: 'image',
      aspect: 'square',
      featuresTiny: true,
      enabled: true,
    })
    expect(calendar.timezone).toBeUndefined()
  })

  it('refuses what would misfire: a bad cron, a bad name, two items with one name', () => {
    expect(() => parseCalendar(JSON.stringify({ items: [{ ...gm, cron: 'every morning' }] }))).toThrow(/cron/)
    expect(() => parseCalendar(JSON.stringify({ items: [{ ...gm, name: 'GM post' }] }))).toThrow(/lowercase/)
    expect(() => parseCalendar(JSON.stringify({ items: [gm, { ...gm, brief: 'another one entirely' }] }))).toThrow(
      /two items are named "gm"/,
    )
    expect(() => parseCalendar(JSON.stringify({ items: [] }))).toThrow()
    expect(() => parseCalendar(JSON.stringify({ items: [{ ...gm, channels: [] }] }))).toThrow()
    expect(() => parseCalendar(JSON.stringify({ items: [{ ...gm, telegramChatIds: ['holders'] }] }))).toThrow()
  })

  it('ships with a calendar that loads', () => {
    const calendar = loadCalendar('calendar.json')
    expect(calendar.items.map((item) => item.name)).toContain('gm')
    expect(calendar.items.every((item) => item.brief.length > 20)).toBe(true)
  })
})
