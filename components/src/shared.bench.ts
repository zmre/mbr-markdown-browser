/**
 * Benchmarks for shared module utilities.
 *
 * Measures URL resolution and canonical path computation performance.
 * Note: siteNav fetch-based logic can't be benchmarked in isolation,
 * so we focus on the pure utility functions.
 */

import { describe, test } from 'vitest'
import * as shared from './shared'

// Bound once to locals: every access to an imported binding goes through the
// module runner's getter, which is measurable at these call rates and makes
// vitest flag the results as unreliable.
const { resolveUrl, getCanonicalPath, getBasePath } = shared

// Each `bench` mutates `window.__MBR_CONFIG__` itself, so no per-iteration
// setup hook is needed.

describe('resolveUrl', () => {
  test('server vs static mode', async ({ bench }) => {
    await bench.compare(
      bench('server mode (absolute path)', () => {
        window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
        resolveUrl('/docs/guide/')
      }),
      bench('static mode (relative path)', () => {
        window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: '../../' }
        resolveUrl('/docs/guide/')
      }),
    )
  })
})

describe('getBasePath', () => {
  test('server vs static mode', async ({ bench }) => {
    await bench.compare(
      bench('server mode', () => {
        window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
        getBasePath()
      }),
      bench('static mode with basePath', () => {
        window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: '../../' }
        getBasePath()
      }),
    )
  })
})

describe('getCanonicalPath', () => {
  test('server vs static mode', async ({ bench }) => {
    await bench.compare(
      bench('server mode', () => {
        window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
        getCanonicalPath()
      }),
      bench('static mode depth 0', () => {
        window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: './' }
        getCanonicalPath()
      }),
      bench('static mode depth 2', () => {
        window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: '../../' }
        getCanonicalPath()
      }),
    )
  })
})
