import { describe, expect, it, vi } from 'vitest'
import { SlotJob } from '../src/jobs/slot.js'

const rota = { days: 7, anchor: '2026-09-14' }
const at = (date: string): number => Date.parse(`${date}T19:00:00Z`)

describe('the evening slot', () => {
  it("runs the day's owner, and the fallback on days nobody has", async () => {
    const article = vi.fn().mockResolvedValue(undefined)
    const serial = vi.fn().mockResolvedValue(undefined)
    const lore = vi.fn().mockResolvedValue(undefined)
    let now = at('2026-09-23') // Wednesday, slot 2
    const slot = new SlotJob({
      label: 'evening',
      rota,
      owners: [
        { name: 'narrative', slots: [1, 4], run: serial },
        { name: 'article', slots: [2, 5], run: article },
      ],
      fallback: { name: 'lore', run: lore },
      logger: { info: vi.fn(), error: vi.fn() },
      now: () => now,
    })
    await slot.run()
    expect(article).toHaveBeenCalledTimes(1)
    now = at('2026-09-25') // Friday, slot 4
    await slot.run()
    expect(serial).toHaveBeenCalledTimes(1)
    now = at('2026-09-27') // Sunday, slot 6
    await slot.run()
    expect(lore).toHaveBeenCalledTimes(1)
    expect(slot.describe()).toBe('0:lore 1:narrative 2:article 3:lore 4:narrative 5:article 6:lore')
  })

  it('is quiet with no owner and no fallback, and survives an owner failing', async () => {
    const logger = { info: vi.fn(), error: vi.fn() }
    const quiet = new SlotJob({ label: 'evening', rota, owners: [{ name: 'article', slots: [2], run: vi.fn() }], logger, now: () => at('2026-09-22') })
    await quiet.run()
    expect(logger.info).toHaveBeenCalledWith(expect.objectContaining({ day: 1 }), 'slot: nobody has today; quiet')
    expect(quiet.describe()).toBe('0:- 1:- 2:article 3:- 4:- 5:- 6:-')

    const failing = new SlotJob({
      label: 'evening',
      rota,
      owners: [{ name: 'article', slots: [2], run: vi.fn().mockRejectedValue(new Error('ladder down')) }],
      logger,
      now: () => at('2026-09-23'),
    })
    await expect(failing.run()).resolves.toBeUndefined()
    expect(logger.error).toHaveBeenCalledWith(expect.objectContaining({ job: 'article' }), "slot: the day's job failed")
  })
})
