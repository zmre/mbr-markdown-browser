import type { UserConfig } from 'vite'
import { resolve } from 'node:path'

// Separate build for the sidebar mini force-graph chunk (`mbr-graph.min.js`).
//
// The main bundle (vite.config.ts) uses `inlineDynamicImports`, which would
// pull d3-force into every page load. Instead, `<mbr-info>` loads this chunk
// on demand the first time the info panel opens (and only when the current
// page has a links.json). The chunk must not import stateful modules like
// `shared.ts` (top-level site.json fetch) — services are injected via element
// properties by the trigger side in the main bundle.
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
      entry: resolve(__dirname, 'src/graph/index.ts'),
      fileName: 'mbr-graph.min',
      name: 'MBRGraph',
      formats: ['es'],
    },
    rollupOptions: {
      output: {
        inlineDynamicImports: true,
      },
    },
  },
} satisfies UserConfig
