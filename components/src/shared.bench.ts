/**
 * Benchmarks for shared module utilities.
 *
 * Measures URL resolution and canonical path computation performance.
 * Note: siteNav fetch-based logic can't be benchmarked in isolation,
 * so we focus on the pure utility functions.
 */

import { bench, describe, beforeEach } from 'vitest'
import { resolveUrl, getCanonicalPath, getBasePath } from './shared'

describe('resolveUrl', () => {
  beforeEach(() => {
    // Reset window config for each bench
    window.__MBR_CONFIG__ = {
      serverMode: true,
      guiMode: false,
    }
  })

  bench('server mode (absolute path)', () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    resolveUrl('/docs/guide/')
  })

  bench('static mode (relative path)', () => {
    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: '../../' }
    resolveUrl('/docs/guide/')
  })
})

describe('getBasePath', () => {
  bench('server mode', () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    getBasePath()
  })

  bench('static mode with basePath', () => {
    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: '../../' }
    getBasePath()
  })
})

describe('getCanonicalPath', () => {
  bench('server mode', () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    getCanonicalPath()
  })

  bench('static mode depth 0', () => {
    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: './' }
    getCanonicalPath()
  })

  bench('static mode depth 2', () => {
    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: '../../' }
    getCanonicalPath()
  })
})
