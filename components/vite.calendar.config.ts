import type { UserConfig } from 'vite'
import { resolve } from 'node:path'
import { minifyLitLiterals, stripPureAnnotations } from './vite-plugins/minify-lit-literals'

// Separate build for the calendar chunk (`mbr-calendar.min.js`): the
// daily-note companion (mini month, day agenda, notes from this day) and the
// agenda time chips; later the calendar overlay and the grids.
//
// `<mbr-calendar>` (main bundle) imports it at idle on periodic notes and on
// notes with time-prefixed bullets, else when `C` or the floating button asks.
// It must not import stateful modules like `shared.ts`: the site index, the
// page fetcher and the URL resolver are injected as element properties.
//
// Unlike the tasks chunk this one SHIPS in static builds: everything it draws
// comes from `site.json` and the pages themselves.
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
      entry: resolve(import.meta.dirname, 'src/calendar/index.ts'),
      fileName: 'mbr-calendar.min',
      name: 'MBRCalendar',
      formats: ['es'],
    },
    rollupOptions: {
      output: {
        codeSplitting: false,
      },
    },
  },
} satisfies UserConfig
