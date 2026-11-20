import type { UserConfig } from 'vite'
import { resolve } from 'node:path'

// Separate build for the standalone PDF export readiness script
// (`mbr-export-ready.min.js`, served at `/.mbr/components/`): for page
// templates that do not load the main bundle (`/.mbr/_export_ready.html`
// includes it). In export mode it imports the same `mbr-export.min.js` chunk
// the main bundle does and posts the same `mbr:export-ready:` message
// (`export-run.ts`); otherwise it does nothing.
//
// IIFE, not a module: a classic `<script defer>` has `document.currentScript`,
// which is how it finds the asset base. It must not import stateful modules
// (`shared.ts`, `dynamic-loader.ts`, the loaders): calendar-chunk.test.ts
// guards that. The chunk is loaded with a runtime `import()` of a URL, which
// Rollup leaves alone, so this stays one small file.
//
// `emptyOutDir: false` so this build appends to the output directory without
// wiping the bundles produced by the earlier `vite build` steps.
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
      entry: resolve(import.meta.dirname, 'src/export-ready-standalone-entry.ts'),
      fileName: () => 'mbr-export-ready.min.js',
      name: 'MBRExportReady',
      formats: ['iife'],
    },
    rollupOptions: {
      output: {
        codeSplitting: false,
      },
    },
  },
} satisfies UserConfig
