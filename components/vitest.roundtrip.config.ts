import { defineConfig } from 'vitest/config'
import base from './vitest.config'

/**
 * `bun run roundtrip-check` (see `scripts/roundtrip-check.ts`): the normal
 * test configuration — happy-dom, the setup file, the defines the editor
 * test support reads — running only the tool's entry, with no time limit.
 * Kept out of `bun run test`, whose `include` is `src/**\/*.test.ts`.
 */
export default defineConfig({
  ...base,
  test: {
    ...base.test,
    include: ['src/roundtrip-check/run.check.ts'],
    testTimeout: 24 * 60 * 60 * 1000,
    pool: 'forks',
    execArgv: ['--max-old-space-size=4096'],
  },
})
