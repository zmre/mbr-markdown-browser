import type { UserConfig } from 'vite'
import { resolve } from 'node:path'

// Separate build for the task-browser panel chunk (`mbr-tasks.min.js`).
//
// The main bundle (vite.config.ts) uses `inlineDynamicImports`, which would
// pull the whole two-pane panel into every page load. Instead, `<mbr-tasks>`
// loads this chunk on demand the first time the task browser is opened. The
// chunk must not import stateful modules like `shared.ts` (top-level site.json
// fetch) — the endpoint and the URL resolver are injected via element
// properties by the trigger side in the main bundle.
//
// Unlike the graph and genealogy chunks, this one is deliberately NOT written
// into static builds: the task index is built from live files, so tasks are
// server/GUI only (TASKS_SPEC.md "Applicability"). See the exclusion in
// `Builder::handle_mbr_folder`.
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
      entry: resolve(__dirname, 'src/tasks/index.ts'),
      fileName: 'mbr-tasks.min',
      name: 'MBRTasks',
      formats: ['es'],
    },
    rollupOptions: {
      output: {
        inlineDynamicImports: true,
      },
    },
  },
} satisfies UserConfig
