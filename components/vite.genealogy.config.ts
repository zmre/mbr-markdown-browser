import type { UserConfig } from 'vite'
import { resolve } from 'node:path'

// Separate build for the person-page genealogy chart chunk
// (`mbr-genealogy.min.js`): family-chart + the custom timeline tree.
//
// The `<mbr-genealogy>` trigger in the main bundle loads this chunk on demand
// only on `type: person` pages (IntersectionObserver-gated), so no other page
// pays for family-chart/d3. CSS is imported `?inline` inside the chunk so this
// build emits a single self-contained JS file.
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
      entry: resolve(__dirname, 'src/genealogy/index.ts'),
      fileName: 'mbr-genealogy.min',
      name: 'MBRGenealogy',
      formats: ['es'],
    },
    rollupOptions: {
      output: {
        inlineDynamicImports: true,
      },
    },
  },
} satisfies UserConfig
