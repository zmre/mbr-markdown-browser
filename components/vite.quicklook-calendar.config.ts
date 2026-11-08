import type { UserConfig } from 'vite'
import { resolve } from 'node:path'

// Separate build for QuickLook's day calendar (`mbr-quicklook-calendar.min.js`,
// design/calendar.md §4.7): a self-contained IIFE that
// `crates/mbr-ffi/src/quicklook.rs` inlines into a preview of a dated note with
// time bullets, after the `window.__MBR_QUICKLOOK_CALENDAR__` payload.
//
// QuickLook only. It is `assets::QUICKLOOK_CALENDAR_JS`, outside
// `DEFAULT_FILES`: never served, never copied into a static build (pages there
// have the calendar chunk), never inlined into a thumbnail. No Lit: it draws
// one static panel from Phase 2's pure calendar modules.
//
// IIFE, not ES: an inlined classic `<script>` cannot be a module, and a
// preview has nowhere to import from.
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
      entry: resolve(import.meta.dirname, 'src/quicklook-calendar/index.ts'),
      fileName: () => 'mbr-quicklook-calendar.min.js',
      name: 'MBRQuickLookCalendar',
      formats: ['iife'],
    },
    rollupOptions: {
      output: {
        codeSplitting: false,
      },
    },
  },
} satisfies UserConfig
