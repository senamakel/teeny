import { describe, expect, it } from 'vitest'
import { loadConfig } from '../src/config.js'

/**
 * A filled-in .env spells "not set" as `KEY=`, so every optional value with a
 * format has to accept the empty spelling. Getting this wrong is not a config
 * warning: `loadConfig` runs at import time, so the process refuses to boot and
 * the deploy's health check is what tells you.
 */
const REQUIRED = {
  X_BOT_USERNAME: 'teenydroid',
  X_API_KEY: 'k',
  X_API_SECRET: 's',
  X_ACCESS_TOKEN: 't',
  X_ACCESS_TOKEN_SECRET: 'ts',
  CORTEX_TOKEN: 'c',
  LLM_BASE_URL: 'http://ladder:6969/v1',
  LLM_API_KEY: 'l',
  LLM_MODEL: 'reasoning',
}

describe('wallet config', () => {
  it('accepts the blanks a half-filled .env actually contains', () => {
    const config = loadConfig({
      ...REQUIRED,
      AIRDROP_ENABLED: 'false',
      AIRDROP_PRIVATE_KEY: '',
      AIRDROP_TOKEN_ADDRESS: '',
      AIRDROP_RPC_URL: '',
    } as unknown as NodeJS.ProcessEnv)

    expect(config.AIRDROP_PRIVATE_KEY).toBeUndefined()
    expect(config.AIRDROP_TOKEN_ADDRESS).toBeUndefined()
  })

  it('is off unless it is switched on', () => {
    expect(loadConfig(REQUIRED as unknown as NodeJS.ProcessEnv).AIRDROP_ENABLED).toBe(false)
  })

  it('still rejects a value that is present and malformed', () => {
    expect(() =>
      loadConfig({
        ...REQUIRED,
        AIRDROP_TOKEN_ADDRESS: '0xnope',
      } as unknown as NodeJS.ProcessEnv),
    ).toThrow()
  })

  it('keeps spending off even when replies are already live', () => {
    const config = loadConfig({
      ...REQUIRED,
      DRY_RUN: 'false',
    } as unknown as NodeJS.ProcessEnv)

    expect(config.DRY_RUN).toBe(false)
    expect(config.AIRDROP_DRY_RUN).toBe(true)
  })
})

describe('chart agent config', () => {
  it('is off unless it is switched on, because it publishes', () => {
    const config = loadConfig(REQUIRED as unknown as NodeJS.ProcessEnv)
    expect(config.CHART_ENABLED).toBe(false)
    expect(config.CHART_RENDERER).toBe('native')
  })

  it('accepts a blank token address rather than refusing to boot', () => {
    const config = loadConfig({
      ...REQUIRED,
      CHART_TOKEN_ADDRESS: '',
      CHART_URL_TEMPLATE: '',
      GMGN_BASE_URL: '',
      SOLANA_RPC_URL: '',
    } as unknown as NodeJS.ProcessEnv)

    expect(config.CHART_TOKEN_ADDRESS).toBeUndefined()
    expect(config.GMGN_BASE_URL).toBeUndefined()
  })

  it('parses the watchlist into wallets and labels', () => {
    const config = loadConfig({
      ...REQUIRED,
      KOL_WALLETS: ' Wallet1:a big fund , Wallet2 ,',
    } as unknown as NodeJS.ProcessEnv)

    expect(config.KOL_WALLETS).toEqual([
      { wallet: 'Wallet1', label: 'a big fund' },
      // An entry with no label still counts: knowing it is on the list is the
      // point, and "watchlist" is a truthful name for it.
      { wallet: 'Wallet2', label: 'watchlist' },
    ])
  })

  it('has an empty watchlist by default, so no wallet is ever mislabelled', () => {
    expect(loadConfig(REQUIRED as unknown as NodeJS.ProcessEnv).KOL_WALLETS).toEqual([])
  })
})

describe('telegram config', () => {
  it('is off when the token is blank, which is how a filled-in .env spells absent', () => {
    const config = loadConfig({ ...REQUIRED, TELEGRAM_BOT_TOKEN: '' } as unknown as NodeJS.ProcessEnv)
    expect(config.TELEGRAM_BOT_TOKEN).toBeUndefined()
    expect(config.TELEGRAM_DRY_RUN).toBe(false)
    expect(config.TELEGRAM_MAX_MESSAGE_AGE_S).toBe(300)
    expect(config.TELEGRAM_HISTORY_TOKENS).toBe(5_000)
  })

  it('accepts a token in the shape BotFather hands out', () => {
    const config = loadConfig({
      ...REQUIRED,
      TELEGRAM_BOT_TOKEN: '8782739916:AAGsGFV_6Rxbng3KmKWp8FgPe0gbiDTis-o',
    } as unknown as NodeJS.ProcessEnv)
    expect(config.TELEGRAM_BOT_TOKEN).toBe('8782739916:AAGsGFV_6Rxbng3KmKWp8FgPe0gbiDTis-o')
  })

  it('rejects something that is not a bot token rather than polling with it', () => {
    expect(() =>
      loadConfig({ ...REQUIRED, TELEGRAM_BOT_TOKEN: 'not-a-token' } as unknown as NodeJS.ProcessEnv),
    ).toThrow()
  })
})

describe('media config', () => {
  it('sees by default, draws and films only when switched on, and points everything at the ladder', () => {
    const config = loadConfig({ ...REQUIRED } as unknown as NodeJS.ProcessEnv)
    expect(config.VISION_MODEL).toBe('vision')
    expect(config.IMAGE_GEN_ENABLED).toBe(false)
    expect(config.VIDEO_GEN_ENABLED).toBe(false)
    expect(config.IMAGE_GEN_MODEL).toBe('image')
    expect(config.VIDEO_GEN_MODEL).toBe('video')
    expect(config.VIDEO_GEN_API).toBe('ladder')
    expect(config.IMAGE_GEN_BASE_URL).toBeUndefined()
    expect(config.TINY_REFERENCE_IMAGES).toHaveLength(10)
    expect(config.TINY_REFERENCE_IMAGES[0]).toMatch(/^assets\/tiny\/.*\.jpg$/)
  })

  it('accepts the blanks a half-filled .env contains, and reads the reference list', () => {
    const config = loadConfig({
      ...REQUIRED,
      VISION_MODEL: '',
      VISION_BASE_URL: '',
      IMAGE_GEN_API_KEY: '',
      IMAGE_GEN_QUALITY: '',
      VIDEO_GEN_BASE_URL: '',
      TINY_REFERENCE_IMAGES: ' https://example.com/a.jpg, assets/tiny/b.jpg ,',
    } as unknown as NodeJS.ProcessEnv)
    expect(config.VISION_MODEL).toBe('')
    expect(config.IMAGE_GEN_QUALITY).toBeUndefined()
    expect(config.TINY_REFERENCE_IMAGES).toEqual(['https://example.com/a.jpg', 'assets/tiny/b.jpg'])

    expect(
      loadConfig({ ...REQUIRED, TINY_REFERENCE_IMAGES: 'none' } as unknown as NodeJS.ProcessEnv)
        .TINY_REFERENCE_IMAGES,
    ).toEqual([])
    expect(
      loadConfig({ ...REQUIRED, TINY_REFERENCE_IMAGES: '' } as unknown as NodeJS.ProcessEnv)
        .TINY_REFERENCE_IMAGES,
    ).toHaveLength(10)
  })
})

describe('content filter config', () => {
  it('runs the judge unless it is switched off, and rides on the chat model by default', () => {
    const config = loadConfig({ ...REQUIRED })
    expect(config.SAFETY_JUDGE).toBe(true)
    expect(config.SAFETY_JUDGE_MODEL).toBeUndefined()
    expect(loadConfig({ ...REQUIRED, SAFETY_JUDGE: 'false' }).SAFETY_JUDGE).toBe(false)
    expect(loadConfig({ ...REQUIRED, SAFETY_JUDGE_MODEL: '' }).SAFETY_JUDGE_MODEL).toBeUndefined()
    expect(loadConfig({ ...REQUIRED, SAFETY_JUDGE_MODEL: 'fast' }).SAFETY_JUDGE_MODEL).toBe('fast')
  })

  it('parses the extra link allowlist and is empty by default', () => {
    expect(loadConfig({ ...REQUIRED }).LINK_ALLOWLIST).toEqual([])
    expect(
      loadConfig({ ...REQUIRED, LINK_ALLOWLIST: 'dexscreener.com/solana, solscan.io ,,' }).LINK_ALLOWLIST,
    ).toEqual(['dexscreener.com/solana', 'solscan.io'])
  })
})
