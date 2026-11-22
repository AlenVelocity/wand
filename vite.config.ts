import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Two pages: the settings window (index.html) and the overlay (overlay.html).
// The overlay is framework-free on purpose: nothing but a WebGL canvas sits
// between a wand sample and the screen.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: '127.0.0.1' },
  build: {
    target: 'chrome120',
    rollupOptions: {
      input: {
        settings: 'index.html',
        overlay: 'overlay.html',
      },
    },
  },
});
