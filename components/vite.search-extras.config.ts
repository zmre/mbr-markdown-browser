import type { UserConfig } from 'vite'
import { resolve } from 'node:path'

// Separate build for the search-panel extras chunk (`mbr-search-extras.min.js`):
// the folder picker element and the note-type / folder derivation.
//
// Every page pays for the main bundle (vite.config.ts uses
// `codeSplitting: false`), and none of this is needed before the search modal
// opens, so `<mbr-search>` imports the chunk on first open instead.
//
// The chunk must not import stateful main-bundle modules (`shared.ts` fetches
// site.json at import time): the site.json payload is passed in.
//
// Like the task and review chunks, it is NOT written into static builds: the
// controls it serves (scope select, folder scope) exist only in server/GUI
// mode, where search goes to the server. See `SEARCH_EXTRAS_CHUNK_ROUTE`.
//
// `emptyOutDir: false` so this build appends to the same output directory
// without wiping the bundles produced by the earlier `vite build` steps.
// `minify: 'terser'` because rolldown-vite does not ship esbuild.
export default {
  build: {
    outDir: '../crates/mbr-core/templates/components-js',
    emptyOutDir: false,
    sourcemap: false,
    target: 'es2020',
    minify: 'terser',
    terserOptions: {
      compress: {
        drop_console: ['log', 'info', 'debug'],
        drop_debugger: true,
        passes: 2,
      },
      mangle: {
        properties: false,
      },
      format: {
        comments: false,
      },
    },
    lib: {
      entry: resolve(import.meta.dirname, 'src/search-extras/index.ts'),
      fileName: 'mbr-search-extras.min',
      name: 'MBRSearchExtras',
      formats: ['es'],
    },
    rollupOptions: {
      output: {
        codeSplitting: false,
      },
    },
  },
} satisfies UserConfig
