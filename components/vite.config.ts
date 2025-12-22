import type { UserConfig } from 'vite'
import { dirname, resolve } from 'node:path'

export default {
  build: {
    sourcemap: true,
    minify: "esbuild",
    lib: {
      entry: resolve(__dirname, 'src/main.js'),
      fileName: 'mbr-components',
      name: 'MBR',
    },
    rollupOptions: {
      // Pagefind is loaded at runtime from static sites, not bundled
      external: ['/.mbr/pagefind/pagefind.js'],
    }
  }
} satisfies UserConfig
