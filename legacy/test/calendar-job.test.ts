import { describe, expect, it, vi } from 'vitest'
import type { ContentDraft } from '../src/agent/content-model.js'
import type { CalendarItem } from '../src/calendar/calendar.js'
import { CalendarJob, type CalendarJobDependencies } from '../src/jobs/calendar-job.js'
import type { Journal } from '../src/journal/journal.js'
import { MediaStore } from '../src/media/store.js'
import { lexicalGate, SafetyGate } from '../src/safety/gate.js'
import type { HoldLog } from '../src/safety/holds.js'

const NOW = Date.UTC(2026, 8, 14, 7, 0)

const gm: CalendarItem = {
  name: 'gm',
  cron: '0 7 * * *',
  channels: ['x', 'telegram'],
  media: 'image',
  aspect: 'square',
  featuresTiny: true,
  enabled: true,
  brief: 'A good-morning post.',
}

function build(
  draft: Partial<ContentDraft>,
  overrides: Partial<CalendarJobDependencies> = {},
) {
  const media = new MediaStore()
  const deps: CalendarJobDependencies = {
    agent: {
      draft: vi.fn().mockResolvedValue({ text: 'gm frens', thinking: 't', declined: [], tools: [], ...draft }),
    },
    memory: {
      recall: vi.fn().mockResolvedValue('yesterday: Tiny with coffee'),
      captureActivity: vi.fn().mockResolvedValue(undefined),
    },
    x: {
      createPost: vi.fn().mockResolvedValue({ id: '77' }),
      uploadMedia: vi.fn().mockResolvedValue({ mediaId: 'm-1' }),
    },
    telegram: {
      sendMessage: vi.fn().mockResolvedValue({ id: 1 }),
      sendPhoto: vi.fn().mockResolvedValue({ id: 2 }),
      sendVideo: vi.fn().mockResolvedValue({ id: 3 }),
    },
    media,
    logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn() },
    dryRun: false,
    telegramChatIds: ['-100'],
    journal: { record: vi.fn().mockResolvedValue(undefined), recent: vi.fn() } as unknown as Journal,
    now: () => NOW,
    ...overrides,
  }
  return { job: new CalendarJob(deps), deps, media }
}

describe('CalendarJob', () => {
  it('posts the item to X and to the Telegram chats with its generated picture, and remembers it', async () => {
    const media = new MediaStore()
    const stored = media.put({ kind: 'image', prompt: 'p', data: new Uint8Array([1]), mimeType: 'image/png' })
    const { job, deps } = build({ mediaId: stored.id }, { media })

    await expect(job.run(gm)).resolves.toBe(true)

    expect(deps.agent.draft).toHaveBeenCalledWith(gm, { today: '2026-09-14', memory: 'yesterday: Tiny with coffee' })
    expect(deps.x.uploadMedia).toHaveBeenCalledWith({ data: new Uint8Array([1]), mimeType: 'image/png' })
    expect(deps.x.createPost).toHaveBeenCalledWith('gm frens', { mediaIds: ['m-1'] })
    expect(deps.telegram!.sendPhoto).toHaveBeenCalledWith(-100, {
      data: new Uint8Array([1]),
      mimeType: 'image/png',
      caption: 'gm frens',
    })
    expect(deps.telegram!.sendMessage).not.toHaveBeenCalled()
    expect(deps.memory.captureActivity).toHaveBeenCalledWith(
      'calendar_post',
      'calendar:gm:2026-09-14',
      expect.objectContaining({ item: 'gm', text: 'gm frens', media: 'image', published: true }),
    )
    expect(deps.journal!.record).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'calendar:gm:2026-09-14', trigger: 'calendar', outcome: 'posted', said: 'gm frens' }),
    )
  })

  it('holds a post that promised a picture and has none, rather than publishing it bare', async () => {
    const { job, deps } = build({})
    await job.run(gm)
    expect(deps.x.createPost).not.toHaveBeenCalled()
    expect(deps.telegram!.sendPhoto).not.toHaveBeenCalled()
    expect(deps.journal!.record).toHaveBeenCalledWith(
      expect.objectContaining({ outcome: 'held', outcomeNote: expect.stringContaining('none was generated') }),
    )

    const { job: other, deps: otherDeps } = build({ mediaId: 'img_gone_1' })
    await other.run(gm)
    expect(otherDeps.x.createPost).not.toHaveBeenCalled()
    expect(otherDeps.journal!.record).toHaveBeenCalledWith(
      expect.objectContaining({ outcome: 'held', outcomeNote: expect.stringContaining('not in the store') }),
    )
  })

  it('sends a text-only item as text, a video as a video, and the item\'s own chats over the defaults', async () => {
    const { job, deps } = build({ text: 'friday, shipped' })
    await job.run({ ...gm, name: 'ship', media: 'none', channels: ['telegram'], telegramChatIds: ['-200', '-201'] })
    expect(deps.x.createPost).not.toHaveBeenCalled()
    expect(deps.telegram!.sendMessage).toHaveBeenCalledWith(-200, 'friday, shipped')
    expect(deps.telegram!.sendMessage).toHaveBeenCalledWith(-201, 'friday, shipped')

    const media = new MediaStore()
    const clip = media.put({ kind: 'video', prompt: 'p', data: new Uint8Array([2]), mimeType: 'video/mp4' })
    const { job: sunday, deps: sundayDeps } = build({ text: '', mediaId: clip.id }, { media })
    await sunday.run({ ...gm, name: 'sunday', media: 'video', channels: ['telegram'] })
    expect(sundayDeps.telegram!.sendVideo).toHaveBeenCalledWith(-100, { data: new Uint8Array([2]), mimeType: 'video/mp4' })
  })

  it('does not publish in dry run, but still remembers and journals what it would have said', async () => {
    const media = new MediaStore()
    const stored = media.put({ kind: 'image', prompt: 'p', data: new Uint8Array([1]), mimeType: 'image/png' })
    const { job, deps } = build({ mediaId: stored.id }, { media, dryRun: true })
    await job.run(gm)
    expect(deps.x.createPost).not.toHaveBeenCalled()
    expect(deps.telegram!.sendPhoto).not.toHaveBeenCalled()
    expect(deps.memory.captureActivity).toHaveBeenCalledWith(
      'calendar_post',
      expect.any(String),
      expect.objectContaining({ published: false }),
    )
    expect(deps.journal!.record).toHaveBeenCalledWith(expect.objectContaining({ outcome: 'dry-run' }))
  })

  it('refuses to run the same item twice inside the window, per item', async () => {
    const { job, deps } = build({ text: 'hi' })
    const text = { ...gm, media: 'none' as const, channels: ['x'] as CalendarItem['channels'] }
    await expect(job.run(text)).resolves.toBe(true)
    await expect(job.run(text)).resolves.toBe(false)
    await expect(job.run({ ...text, name: 'other' })).resolves.toBe(true)
    await expect(job.run(text, { force: true })).resolves.toBe(true)
    expect(deps.x.createPost).toHaveBeenCalledTimes(3)
  })

  it('keeps going when one channel fails, and records the failure', async () => {
    const { job, deps } = build({ text: 'hi' })
    ;(deps.x.createPost as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('duplicate content'))
    await job.run({ ...gm, media: 'none' })
    expect(deps.telegram!.sendMessage).toHaveBeenCalledWith(-100, 'hi')
    expect(deps.journal!.record).toHaveBeenCalledWith(
      expect.objectContaining({ outcome: 'posted', outcomeNote: expect.stringContaining('duplicate content') }),
    )

    const { job: nowhere, deps: nowhereDeps } = build({ text: 'hi' }, { telegramChatIds: [] })
    await nowhere.run({ ...gm, media: 'none', channels: ['telegram'] })
    expect(nowhereDeps.journal!.record).toHaveBeenCalledWith(expect.objectContaining({ outcome: 'failed' }))
  })

  it('withholds a draft the content filter refuses, on every channel, and journals the hold without the text', async () => {
    const record = vi.fn().mockResolvedValue(undefined)
    const holds = { record, allowedLinks: vi.fn().mockResolvedValue([]) } as unknown as HoldLog
    const { job, deps } = build({ text: 'best gun for home defence? tiny knows' }, { safety: lexicalGate({ holds }) })
    await job.run({ ...gm, media: 'none' })

    expect(deps.x.createPost).not.toHaveBeenCalled()
    expect(deps.telegram!.sendMessage).not.toHaveBeenCalled()
    expect(deps.memory.captureActivity).not.toHaveBeenCalled()
    expect(deps.journal!.record).toHaveBeenCalledWith(
      expect.objectContaining({ outcome: 'held', said: '', outcomeNote: expect.stringContaining('content filter withheld') }),
    )
    expect(record).toHaveBeenCalledWith(
      expect.objectContaining({
        direction: 'outbound',
        kind: 'draft',
        context: { surface: 'calendar', kind: 'draft', tool: 'gm' },
        category: 'weapons',
      }),
    )
  })

  it('publishes what the filter handed back, with a link off the allowlist cut', async () => {
    const { job, deps } = build({ text: 'gm frens, see https://totally-legit-claim.xyz/airdrop for more' }, { safety: lexicalGate() })
    await job.run({ ...gm, media: 'none', channels: ['x'] })
    expect(deps.x.createPost).toHaveBeenCalledWith('gm frens, see for more', {})
    expect(deps.logger.warn).toHaveBeenCalledWith(
      expect.objectContaining({ item: 'gm', fixes: [expect.stringContaining('cut 1 link')] }),
      expect.stringContaining('content filter edited'),
    )
  })

  it('asks the judge about the draft and fails closed on its verdict', async () => {
    const judge = vi.fn().mockResolvedValue({ verdict: 'prohibited', category: 'sexual', reason: 'suggestive' })
    const { job, deps } = build({ text: 'gm frens' }, { safety: new SafetyGate({ judge: { judge } }) })
    await job.run({ ...gm, media: 'none' })
    expect(judge).toHaveBeenCalledWith({ text: 'gm frens', direction: 'outbound' })
    expect(deps.x.createPost).not.toHaveBeenCalled()
    expect(deps.telegram!.sendMessage).not.toHaveBeenCalled()

    const clean = vi.fn().mockResolvedValue({ verdict: 'clean', reason: '' })
    const { job: ok, deps: okDeps } = build({ text: 'gm frens' }, { safety: new SafetyGate({ judge: { judge: clean } }) })
    await ok.run({ ...gm, media: 'none', channels: ['x'] })
    expect(okDeps.x.createPost).toHaveBeenCalledWith('gm frens', {})
  })

  it('journals a drafter that threw and does not publish', async () => {
    const { job, deps } = build({})
    ;(deps.agent.draft as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('model down'))
    await job.run(gm)
    expect(deps.x.createPost).not.toHaveBeenCalled()
    expect(deps.journal!.record).toHaveBeenCalledWith(
      expect.objectContaining({ outcome: 'failed', outcomeNote: 'model down' }),
    )
  })
})
