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
    sourcemap: false, // Disable sourcemaps in production for smaller bundle
    minify: 'terser', // Use terser for more aggressive minification than esbuild
    terserOptions: {
      compress: {
        drop_console: true,  // Remove console.* statements
        drop_debugger: true, // Remove debugger statements
        passes: 2,           // Run compression twice for better results
      },
      mangle: {
        properties: false,   // Don't mangle property names (breaks Lit)
      },
      format: {
        comments: false,     // Remove all comments
      },
    },
    lib: {
      entry: resolve(__dirname, 'src/main.js'),
      fileName: 'mbr-components.min',
      name: 'MBR',
      // Use 'es' format without code splitting for simpler embedding
      formats: ['es'],
    },
    rollupOptions: {
      // Pagefind is loaded at runtime from static sites, not bundled
      external: ['/.mbr/pagefind/pagefind.js'],
      output: {
        // Disable code splitting - bundle everything into one file
        // This is essential since we serve the bundle as a single embedded file
        inlineDynamicImports: true,
      },
    }
  }
} satisfies UserConfig
