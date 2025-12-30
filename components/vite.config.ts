import type { UserConfig } from 'vite'
import { resolve } from 'node:path'
import { existsSync } from 'node:fs'

export default {
  server: {
    fs: {
      allow: ['..'],
    },
  },
  optimizeDeps: {
    // Don't try to pre-bundle pagefind - it's loaded at runtime
    exclude: ['/.mbr/pagefind/pagefind.js'],
  },
  plugins: [
    {
      name: 'mbr-external-pagefind',
      enforce: 'pre',
      resolveId(id) {
        // Tell Vite this is an external module that will be resolved at runtime
        if (id === '/.mbr/pagefind/pagefind.js') {
          return { id, external: true };
        }
      },
    },
    {
      name: 'mbr-dev-assets',
      configureServer(server) {
        server.middlewares.use((req, res, next) => {
          // Serve pagefind mock from mocks/ folder
          if (req.url?.startsWith('/.mbr/pagefind/pagefind.js')) {
            req.url = '/mocks/pagefind.js';
            return next();
          }

          // Serve other /.mbr/* from templates (CSS, etc.)
          if (req.url?.startsWith('/.mbr/')) {
            const filePath = req.url.slice(6); // Remove '/.mbr/' prefix
            const templatePath = resolve(__dirname, '..', 'templates', filePath);
            if (existsSync(templatePath)) {
              req.url = '/../templates/' + filePath;
            }
          }
          next();
        });
      },
    },
  ],
  build: {
    outDir: '../templates/components-js',
    emptyOutDir: true,
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
