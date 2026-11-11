import type { UserConfig } from 'vite'
import { resolve } from 'node:path'

// Separate build for the not-found chunk (`mbr-not-found.min.js`).
//
// "Where did it go?": the matcher over site.json and the 404 page's renderer.
// `<mbr-not-found-suggestions>` imports it on 404 pages only, and
// `<mbr-page-errors>` when its panel lists broken links (for "Did you mean").
//
// Imports no main-bundle module, stateful or not: site data and `resolveUrl`
// are passed in. Written into static builds, whose `404.html` uses it.
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
      entry: resolve(import.meta.dirname, 'src/not-found/index.ts'),
      fileName: 'mbr-not-found.min',
      name: 'MBRNotFound',
      formats: ['es'],
    },
    rollupOptions: {
      output: {
        codeSplitting: false,
      },
    },
  },
} satisfies UserConfig
