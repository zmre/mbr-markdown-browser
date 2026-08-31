import type { UserConfig } from 'vite'
import { resolve } from 'node:path'

// Separate build for the review-notes panel chunk (`mbr-review.min.js`).
//
// The main bundle (vite.config.ts) uses `inlineDynamicImports`, which would pull
// the panel and the note form into every page load. Instead `<mbr-review>` — a
// trigger, the in-document markers and the keyboard shortcuts, all small — loads
// this chunk the first time a note is written or the list is opened.
//
// The chunk must not import stateful main-bundle modules: `review-store.ts`
// (the note cache and its subscribers), `shared.ts` (top-level site.json fetch)
// or `task-toggle.ts` (the source-line cache). A second copy of any of them
// inside the chunk would be a second, silently divergent instance. The store,
// the source reader and the URL resolver are injected as element properties by
// the trigger side in the main bundle.
//
// Like the task chunk and for the same reason, this one is deliberately NOT
// written into static builds: `data-mbr-line` is emitted only in server/GUI
// mode, so a static page has nothing to anchor a note to and `<mbr-review>`
// never renders there. See the exclusion in `Builder::handle_mbr_folder`.
//
// `emptyOutDir: false` so this build appends to the same output directory
// without wiping the bundles produced by the earlier `vite build` steps.
// `minify: 'terser'` because rolldown-vite does not ship esbuild.
export default {
  build: {
    outDir: '../templates/components-js',
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
      entry: resolve(__dirname, 'src/review/index.ts'),
      fileName: 'mbr-review.min',
      name: 'MBRReview',
      formats: ['es'],
    },
    rollupOptions: {
      output: {
        inlineDynamicImports: true,
      },
    },
  },
} satisfies UserConfig
