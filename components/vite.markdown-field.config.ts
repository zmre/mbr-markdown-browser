import type { UserConfig } from 'vite'
import { resolve } from 'node:path'
import editorConfig from './vite.editor.config'

// Separate build for `<mbr-markdown-field>` (`mbr-markdown-field.min.js`): the
// editor as a form field for pages mbr did not render (a host app loads it
// from `/.mbr/components/`). Entry `src/markdown-field/index.ts`.
//
// Everything but the entry is the editor chunk's build (vite.editor.config.ts):
// the same Crepe graph needs the same `@codemirror/language-data` stub,
// `process.env.NODE_ENV` define and terser settings, and Crepe's CSS is again
// inlined, so the output is one self-contained ES module. It shares source with
// `mbr-editor.min.js` (`editor-core.ts`) but neither imports the other: a host
// page loads only this file.
export default {
  ...editorConfig,
  build: {
    ...editorConfig.build,
    lib: {
      entry: resolve(import.meta.dirname, 'src/markdown-field/index.ts'),
      fileName: 'mbr-markdown-field.min',
      name: 'MBRMarkdownField',
      formats: ['es'],
    },
  },
} satisfies UserConfig
