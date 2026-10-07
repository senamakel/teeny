import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

/**
 * The TypeScript service lives in `legacy/` while the Rust rewrite takes the
 * repo root, but the content it reads -- `calendar.json`, `assets/`, `SOUL.md`,
 * `skills/` -- stays at the root, and the config defaults name it relative to
 * the working directory. Tests therefore run from the root, the same place the
 * service and its scripts run from.
 */
process.chdir(resolve(dirname(fileURLToPath(import.meta.url)), '..'))
