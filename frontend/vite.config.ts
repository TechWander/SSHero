import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// dev server proxies /api to the Rust backend so browser same-origin rules
// don't get in the way; production build is served by the backend directly.
export default defineConfig({
  plugins: [vue()],
  server: {
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:8901',
        changeOrigin: true,
      },
    },
  },
  build: {
    chunkSizeWarningLimit: 600,
  },
})
