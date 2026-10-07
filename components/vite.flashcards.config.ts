import type { UserConfig } from 'vite'
import { resolve } from 'node:path'

// Separate build for the flashcard review overlay (`mbr-flashcards.min.js`).
//
// The main bundle (vite.config.ts) uses `codeSplitting: false`, which would pull
// the overlay and ts-fsrs into every page load. Instead `<mbr-flashcards>` — a
// nav button and the `p` key, nothing more — loads this chunk the first time a
// deck is opened. (The reading view's history summaries are a third, much
// smaller chunk: `vite.flashcards-reading.config.ts`.)
//
// The chunk must not import stateful main-bundle modules: `shared.ts`,
// `task-toggle.ts` (the source-line cache and the self-write window) or
// `edit-token.ts`. The review writer lives here, but builds itself from those
// modules' functions, which the trigger passes to `makeReviewRecorder`.
//
// Unlike the task and review chunks this one IS written into static builds:
// "In order" and "Random" review need nothing from the server. Spaced
// repetition is offered only when editing is enabled.
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
      entry: resolve(import.meta.dirname, 'src/flashcards/index.ts'),
      fileName: 'mbr-flashcards.min',
      name: 'MBRFlashcards',
      formats: ['es'],
    },
    rollupOptions: {
      output: {
        codeSplitting: false,
      },
    },
  },
} satisfies UserConfig
