import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { fileURLToPath } from 'node:url';
import { createSsrLoader } from '../ssr.mjs';

const production = process.argv.includes('--production');
if (production) process.env.NODE_ENV = 'production';

const root = fileURLToPath(new URL('.', import.meta.url));
const origin = 'http://127.0.0.1:5173';
const loader = production ? null : createSsrLoader({ root, origin });
const bundle = production ? await import('./dist/server/entry.mjs') : null;
const assets = production
  ? JSON.parse(await readFile(new URL('./dist/ssr-manifest.json', import.meta.url), 'utf8'))['index.html']
  : { script: '/entry-client.ts', css: [] };

function document(app) {
  const absolute = url => url.replace(/^\./, '');
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Rush SSR</title>
    ${assets.css.map(url => `<link rel="stylesheet" href="${absolute(url)}">`).join('\n    ')}
    ${production ? '' : '<script type="module" src="/__hmr.js"></script>'}
  </head>
  <body>
    <header>${production ? 'PRODUCTION · HASHED ASSETS' : 'DEVELOPMENT · LIVE UPDATES'}</header>
    <div id="app">${app}</div>
    <script type="module" src="${absolute(assets.script)}"></script>
  </body>
</html>`;
}

const server = createServer(async (request, response) => {
  const { pathname } = new URL(request.url, origin);
  try {
    if (pathname === '/') {
      const { render } = bundle ?? await loader.ssrLoadModule('entry-server.mts');
      const html = document(render());
      response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
      response.end(html);
    } else if (production && /^\/assets\/[\w.-]+\.(js|css)(\.map)?$/.test(pathname)) {
      const body = await readFile(new URL(`./dist${pathname}`, import.meta.url));
      const type = pathname.endsWith('.map') ? 'application/json'
        : pathname.endsWith('.css') ? 'text/css' : 'text/javascript';
      response.writeHead(200, { 'Content-Type': type });
      response.end(body);
    } else {
      response.writeHead(404).end();
    }
  } catch (error) {
    if (error.code === 'ENOENT') {
      response.writeHead(404).end();
    } else {
      console.error(error);
      response.writeHead(500).end('SSR render failed');
    }
  }
});

server.listen(4173, '127.0.0.1', () => {
  console.log(`Open ${production ? 'http://127.0.0.1:4173/' : origin + '/ssr/'}`);
});
for (const signal of ['SIGINT', 'SIGTERM']) {
  process.once(signal, () => server.close(() => {
    loader?.close().catch(console.error);
  }));
}
