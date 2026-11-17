import type { UserConfig } from 'vite'
import { resolve } from 'node:path'
import { minifyLitLiterals, stripPureAnnotations } from './vite-plugins/minify-lit-literals'

// Separate build for the PDF export chunk (`mbr-export.min.js`): readies a
// page for paper (`@page` rule, export.css, eager images, link policy) and
// waits until diagrams, math, code highlighting, images and fonts are done.
//
// `export-ready.ts` (main bundle) imports it only when Rust set
// `window.__MBR_EXPORT__` (`mbr --pdf`, Save as PDF…), so no reader's page
// fetches it. It must not import stateful modules like `shared.ts` or the
// loaders: their settle promises and the asset base are injected
// (`ExportDeps`). Ships in static builds like every always-on asset, though
// only the exporter's own server ever loads it.
//
// `emptyOutDir: false` so this build appends to the same output directory
// without wiping the bundles produced by the earlier `vite build` steps.
// `minify: 'terser'` because rolldown-vite does not ship esbuild.
// `minifyLitLiterals` / `stripPureAnnotations` (vite-plugins/): Lit template
// whitespace and the pure annotations a final asset never needs.
export default {
  plugins: [minifyLitLiterals(), stripPureAnnotations()],
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
      entry: resolve(import.meta.dirname, 'src/export/index.ts'),
      fileName: 'mbr-export.min',
      name: 'MBRExport',
      formats: ['es'],
    },
    rollupOptions: {
      output: {
        codeSplitting: false,
      },
    },
  },
} satisfies UserConfig
