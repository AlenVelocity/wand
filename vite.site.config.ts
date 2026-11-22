import { readFileSync, writeFileSync } from 'node:fs';
import { defineConfig } from 'vite';

// The landing page (site/). It imports the overlay renderer and the built-in
// shapes straight from src/, so the page shows the real trail. Output is a
// static folder, site/dist, that any host can serve.
const { version } = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8')) as { version: string };

export default defineConfig({
  root: 'site',
  clearScreen: false,
  server: { port: 1421, strictPort: true, host: '127.0.0.1', fs: { allow: ['..'] } },
  build: { outDir: 'dist', emptyOutDir: true, target: 'es2022' },
  plugins: [
    {
      // The download links and the version shown follow package.json.
      name: 'wand-version',
      transformIndexHtml: { order: 'pre', handler: (html) => html.replaceAll('__VERSION__', version) },
    },
    {
      // Development only: /og.html renders the link-preview card and posts it
      // here to be saved as public/og.png. The header and the origin check
      // keep any other page open in the browser from writing to it.
      name: 'wand-og',
      apply: 'serve',
      configureServer(server) {
        server.middlewares.use('/__og', (req, res) => {
          const local = req.headers.origin === 'http://localhost:1421' || req.headers.origin === 'http://127.0.0.1:1421';
          if (req.method !== 'POST' || req.headers['x-wand-og'] !== '1' || !local) {
            res.statusCode = 403;
            res.end();
            return;
          }
          const chunks: Buffer[] = [];
          req.on('data', (chunk: Buffer) => chunks.push(chunk));
          req.on('end', () => {
            const png = Buffer.concat(chunks);
            // A PNG and nothing else, of a size a card could plausibly be.
            const ok = png.length < 4_000_000 && png.subarray(0, 8).toString('hex') === '89504e470d0a1a0a';
            if (ok) writeFileSync(new URL('./site/public/og.png', import.meta.url), png);
            res.statusCode = ok ? 204 : 400;
            res.end();
          });
        });
      },
    },
  ],
});
