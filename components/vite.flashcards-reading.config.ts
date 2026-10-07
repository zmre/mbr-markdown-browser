import type { UserConfig } from 'vite'
import { resolve } from 'node:path'

// Separate build for the flashcard reading-view chunk (`mbr-flashcards-reading.min.js`).
//
// The history parser and the `<details>` summary for `___Review History___`
// definitions. `<mbr-flashcards>` imports it at idle on `type: flashcard` pages
// only, so no other page pays for it — and it is separate from the deck chunk
// (`vite.flashcards.config.ts`) so that merely *reading* a flashcard note never
// fetches the overlay and ts-fsrs.
//
// Pure DOM work: it imports no main-bundle module, stateful or not. Written
// into static builds, where the summary is wanted just the same.
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
      entry: resolve(import.meta.dirname, 'src/flashcards/reading.ts'),
      fileName: 'mbr-flashcards-reading.min',
      name: 'MBRFlashcardsReading',
      formats: ['es'],
    },
    rollupOptions: {
      output: {
        codeSplitting: false,
      },
    },
  },
} satisfies UserConfig
