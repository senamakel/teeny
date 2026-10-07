import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    /**
     * `worktrees/` holds separate git checkouts of this same repo, each with a
     * full copy of `test/`. Without this the root suite collects both and
     * reports 477 tests where there are 235, half of them running against
     * whatever code that branch happens to be on -- so a green run here would
     * stop meaning this branch is green.
     */
    exclude: ['**/node_modules/**', '**/dist/**', 'worktrees/**'],
    setupFiles: ['./vitest.setup.ts'],
  },
})
