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
