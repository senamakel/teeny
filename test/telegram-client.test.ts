import { describe, expect, it, vi } from 'vitest'
import { TelegramApiClient, TelegramApiError, splitForTelegram } from '../src/clients/telegram.js'

function fetchReturning(body: unknown, status = 200): typeof fetch {
  return vi.fn().mockResolvedValue({
    status,
    json: async () => body,
  }) as unknown as typeof fetch
}

describe('TelegramApiClient', () => {
  it('posts JSON to the method url and unwraps the result', async () => {
    const doFetch = fetchReturning({ ok: true, result: { id: 1, is_bot: true, first_name: 'T' } })
    const client = new TelegramApiClient({ token: '1:abc', fetch: doFetch })

    await expect(client.getMe()).resolves.toEqual({ id: 1, is_bot: true, first_name: 'T' })
    expect(doFetch).toHaveBeenCalledWith(
      'https://api.telegram.org/bot1:abc/getMe',
      expect.objectContaining({ method: 'POST' }),
    )
  })

  it('sends plain text with previews off, threaded when asked', async () => {
    const doFetch = fetchReturning({ ok: true, result: { message_id: 7 } })
    const client = new TelegramApiClient({ token: '1:abc', fetch: doFetch })

    await expect(client.sendMessage(-100, 'hi', { replyToMessageId: 3 })).resolves.toEqual({ id: 7 })
    const [, init] = (doFetch as ReturnType<typeof vi.fn>).mock.calls[0] as [string, RequestInit]
    expect(JSON.parse(init.body as string)).toEqual({
      chat_id: -100,
      text: 'hi',
      reply_to_message_id: 3,
      link_preview_options: { is_disabled: true },
    })
  })

  it('asks only for messages, and waits past the long-poll timeout before giving up', async () => {
    const doFetch = fetchReturning({ ok: true, result: [] })
    const client = new TelegramApiClient({ token: '1:abc', fetch: doFetch })

    await client.getUpdates({ offset: 42, timeoutSeconds: 30 })
    const [, init] = (doFetch as ReturnType<typeof vi.fn>).mock.calls[0] as [string, RequestInit]
    expect(JSON.parse(init.body as string)).toEqual({
      offset: 42,
      timeout: 30,
      allowed_updates: ['message'],
    })
    expect(init.signal).toBeInstanceOf(AbortSignal)
  })

  it('turns an API error into one that carries the code and retry hint', async () => {
    const doFetch = fetchReturning(
      { ok: false, error_code: 429, description: 'Too Many Requests', parameters: { retry_after: 7 } },
      429,
    )
    const client = new TelegramApiClient({ token: '1:abc', fetch: doFetch })

    const error = await client.getMe().catch((e: unknown) => e)
    expect(error).toBeInstanceOf(TelegramApiError)
    expect((error as TelegramApiError).code).toBe(429)
    expect((error as TelegramApiError).retryAfterSeconds).toBe(7)
  })
})

describe('splitForTelegram', () => {
  it('leaves a short message alone', () => {
    expect(splitForTelegram('hello')).toEqual(['hello'])
    expect(splitForTelegram('   ')).toEqual([])
  })

  it('splits at a paragraph break before a word break', () => {
    const text = `${'a'.repeat(30)}\n\n${'b'.repeat(30)} ${'c'.repeat(30)}`
    expect(splitForTelegram(text, 70)).toEqual(['a'.repeat(30), `${'b'.repeat(30)} ${'c'.repeat(30)}`])
  })

  it('never produces a piece over the limit', () => {
    const text = Array.from({ length: 200 }, (_, i) => `word${i}`).join(' ')
    for (const piece of splitForTelegram(text, 100)) expect(piece.length).toBeLessThanOrEqual(100)
  })
})
