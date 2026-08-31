import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitest/config'

/**
 * `templates/theme.css`, read at config time and handed to the tests as a
 * constant.
 *
 * `icons.test.ts` needs it: the six review-note icons are defined in CSS so the
 * in-document markers and the panel badge share one copy, and only a test that
 * reads both ends catches a renamed custom property — which otherwise shows up
 * as a blank marker with no error anywhere.
 *
 * Not imported with Vite's `?raw` because vitest runs with `css: false`, which
 * stubs every CSS import to an empty string; the test would then pass
 * vacuously against nothing. Config files run in Node, so this reads it
 * directly and cannot be silently emptied.
 */
const themeCss = readFileSync(
  fileURLToPath(new URL('../templates/theme.css', import.meta.url)),
  'utf8'
)

export default defineConfig({
  define: {
    __MBR_THEME_CSS__: JSON.stringify(themeCss),
  },
  test: {
    environment: 'happy-dom',
    include: ['src/**/*.test.ts'],
    setupFiles: ['./src/test-setup.ts'],
    benchmark: {
      include: ['src/**/*.bench.ts'],
    },
  },
})
