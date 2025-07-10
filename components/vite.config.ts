import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'

// https://vite.dev/config/
export default defineConfig({
  plugins: [svelte()],
  build: {
    lib: {
      // entry: 'src/main.ts',
      entry: ['src/components/Link.svelte'],
      fileName: (format, entryName) => `mbr-${entryName.toLowerCase()}.${format}.js`,
      name: "components",
      formats: ['iife'],
    }
  },
})
