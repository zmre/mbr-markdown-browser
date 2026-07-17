import type { UserConfig } from 'vite'
import { resolve } from 'node:path'

// Separate build for the heavy Milkdown/Crepe editor chunk.
//
// The main bundle (vite.config.ts) uses `inlineDynamicImports`, which would
// pull Crepe into every page load. Instead, the `<mbr-editor>` trigger loads
// this chunk on demand at runtime. Crepe's CSS is imported as `?inline`
// strings inside the chunk, so this build emits a single self-contained JS file
// (`mbr-editor.min.js`) with no separate stylesheet to embed.
//
// `emptyOutDir: false` so this build appends to the same output directory
// without wiping the main bundle produced by the first `vite build`.
//
// Notes on reliability for this dependency graph:
//   - `resolve.alias` of `@codemirror/language-data` → an empty stub: Crepe
//     statically imports this package (≈50 lazy `@codemirror/lang-*` dynamic
//     imports) and only uses it for its CodeMirror feature, which we disable in
//     editor-crepe.ts. Stubbing it removes that whole graph — the bulk of the
//     bundle size and the inlined dynamic-import init-order breakage.
//   - `define` of `process.env.NODE_ENV`: Crepe depends on Vue's esm-bundler
//     build, which references it and misbehaves if a bundler leaves it
//     unreplaced (library mode does not inject it automatically).
//   - `minify: 'terser'`: matches the main bundle. This project runs on
//     rolldown-vite, which does NOT ship esbuild, so `minify: 'esbuild'` fails;
//     terser is installed and proven here.
export default {
  define: {
    'process.env.NODE_ENV': JSON.stringify('production'),
  },
  resolve: {
    alias: {
      '@codemirror/language-data': resolve(
        __dirname,
        'src/editor-stubs/codemirror-language-data.ts',
      ),
    },
  },
  build: {
    outDir: '../templates/components-js',
    emptyOutDir: false,
    sourcemap: false,
    target: 'es2020',
    minify: 'terser',
    terserOptions: {
      compress: {
        // Leave console.warn/error so real editor failures still surface.
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
      entry: resolve(__dirname, 'src/editor-crepe.ts'),
      fileName: 'mbr-editor.min',
      name: 'MBREditor',
      formats: ['es'],
    },
    rollupOptions: {
      output: {
        inlineDynamicImports: true,
      },
    },
  },
} satisfies UserConfig
