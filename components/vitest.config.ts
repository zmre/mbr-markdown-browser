import { readFileSync, readdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitest/config'

/**
 * `crates/mbr-core/templates/theme.css`, read at config time and handed to the
 * tests as a constant.
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
  fileURLToPath(new URL('../crates/mbr-core/templates/theme.css', import.meta.url)),
  'utf8'
)

/**
 * Two more files outside `components/`, read the same way (Vite's `?raw` is
 * denied outside its root): `_head.html`, whose viewport tag the narrow-screen
 * tests check, and the iOS app's `AppBridge.swift`, whose string literals are
 * the other half of the bridge contract `app-bridge.test.ts` pins.
 */
const readRepoFile = (relative: string): string =>
  readFileSync(fileURLToPath(new URL(relative, import.meta.url)), 'utf8')

/**
 * The editor parity corpus, `tests/fixtures/editor-roundtrip/`: every `*.md`
 * fixture with its renderer outline golden (`*.outline.json`, written by
 * mbr-core's `editor_parity` test), keyed by fixture name. Read byte for byte
 * — `crlf.md` must reach the test with its `\r\n`s. A fixture without a
 * golden maps to `null`, which the parity test reports as a failure.
 */
const readEditorFixtures = (): Record<string, { source: string; outline: string | null }> => {
  const dir = fileURLToPath(new URL('../tests/fixtures/editor-roundtrip/', import.meta.url))
  const fixtures: Record<string, { source: string; outline: string | null }> = {}
  for (const file of readdirSync(dir).filter((f) => f.endsWith('.md')).sort()) {
    const name = file.slice(0, -'.md'.length)
    let outline: string | null = null
    try {
      outline = readFileSync(`${dir}${name}.outline.json`, 'utf8')
    } catch {
      outline = null
    }
    fixtures[name] = { source: readFileSync(`${dir}${file}`, 'utf8'), outline }
  }
  return fixtures
}

export default defineConfig({
  define: {
    __MBR_THEME_CSS__: JSON.stringify(themeCss),
    __MBR_HEAD_TEMPLATE__: JSON.stringify(readRepoFile('../crates/mbr-core/templates/_head.html')),
    __MBR_APP_BRIDGE_SWIFT__: JSON.stringify(readRepoFile('../apple/ios/MBRMobile/Web/AppBridge.swift')),
    __MBR_EDITOR_FIXTURES__: JSON.stringify(readEditorFixtures()),
    // Shared with mbr-core's `editor_math_parity`; under tests/fixtures/ so the
    // Nix test sandbox (flake.nix's source filter) has it too.
    __MBR_EDITOR_MATH_CASES__: JSON.stringify(readRepoFile('../tests/fixtures/editor-math-cases.json')),
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
