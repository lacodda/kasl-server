import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath } from 'node:url'

// The built app is embedded in the server binary (ADR 0012), so `base` stays
// relative-free and `dist` is what `rust-embed` picks up.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  // No version is injected here on purpose: the product's version is the
  // server's, and the UI asks `/health` for it (ADR 0012).
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  server: {
    port: 5173,
    strictPort: true,
    // In development the SPA is served by Vite and the API by the server, so
    // the browser sees one origin and the session cookie behaves as it will in
    // production. Without this, every request would be cross-origin and the
    // cookie would silently not be sent.
    proxy: {
      '/api': { target: 'http://127.0.0.1:8080', changeOrigin: false },
    },
  },
})
