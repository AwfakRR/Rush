<p align="center">
  <img src="assets/logo.png" alt="Rush Logo" width="150" />
</p>

<h1 align="center">Rush</h1>

<p align="center">
  <b>A blazingly fast, minimalist dev server and build tool written in Rust.</b><br/>
  Engineered from first principles: the simplest, shortest, and most efficient solution that works.
</p>

<p align="center">
  <a href="#features">Features</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#cli-options">CLI Options</a> •
  <a href="#example-project">Example</a> •
  <a href="#architecture--design-philosophy">Architecture & Philosophy</a> •
  <a href="#testing">Testing</a> •
  <a href="#benchmarks">Benchmarks</a> •
  <a href="#roadmap">Roadmap</a> •
  <a href="#license">License</a>
</p>

---

Rush provides instant local web development with on-the-fly TypeScript/JSX transpilation, debounced file watching, WebSocket-based live reload, CSS hot swapping, and a production bundler — all without the overhead of heavy JavaScript runtimes or multi-megabyte dependency trees.

## Features

- **⚡ Sub-millisecond Transpilation**: Instant on-the-fly TypeScript (`.ts`), TSX (`.tsx`), and JSX (`.jsx`) transpilation powered by [`oxc`](https://github.com/oxc-project/oxc) — the high-performance Rust toolchain behind Rolldown.
- **🔄 Live Reload & CSS Hot Swap**: WebSocket connection automatically reloads the browser on file saves. Changes to `.css` files hot-swap stylesheets instantly without losing page state or triggering a full refresh.
- **🔥 Granular Module HMR & React Fast Refresh**: Supports `import.meta.hot.accept()` and dependency graph boundary resolution to update individual JS/TS modules in-place, plus state-preserving React component refresh.
- **🧩 Vue, Svelte & Solid**: Compile Vue 3 single-file components, Svelte 5 components, and Solid JSX in both the dev server and production builds.
- **📦 Bare Import Resolution & Pre-bundling**: Rewrites bare npm specifiers (`import React from 'react'`) and automatically pre-bundles CommonJS/ESM dependencies with `esbuild`.
- **🛠️ Self-Healing Auto-Install**: Automatically detects your package manager (`pnpm`, `yarn`, `bun`, or `npm`) and installs missing dependencies on the fly.
- **🌐 HTML5 History API Routing**: Built-in SPA fallback routing so React Router, Vue Router, and SvelteKit routes work seamlessly without 404s.
- **🔒 Zero-Config HTTPS**: On-demand self-signed TLS certificate generation in memory via `rcgen` and `axum-server`.
- **🗺️ Standard v3 Source Maps**: Generates and serves in-memory source maps (`.map`) for seamless in-browser debugging of original TypeScript source.
- **📄 JSON, Environment & Asset Modules**: Built-in `.env` parsing with `RUSH_` filtering, JSON module imports, and static media URL exports.
- **🏗️ Fast Production Builds**: Single-command `rush build` leveraging `esbuild` to bundle, minify, and hash assets into a self-contained `dist/` folder in hundreds of milliseconds.
- **🛡️ Debounced File Watching**: 50ms debouncer coalesces IDE and editor atomic saves into single clean reload triggers.
- **✂️ Minimal Dependency Footprint**: Prioritizes standard library APIs and native platform capabilities over dependency bloat.

---

## Quick Start

### Prerequisites
- [Rust](https://rustup.rs/) (1.96.0+ recommended)

### Build & Run

Clone the repository and run:

```bash
# Serve the current directory on default port 3000
cargo run

# Serve a specific folder on a custom port and auto-open browser
npm ci --prefix example
cargo run -- --root ./example --port 5173 --open
```

Build an optimized release binary:

```bash
cargo build --release
./target/release/rush --root ./public --port 8080
```

---

## CLI Options

| Command / Flag | Short | Default | Description |
|---|---|---|---|
| `rush build` | - | - | Bundle project for production into `dist/` |
| `rush build --lib [--entry <path>]` | - | - | Bundle a library and emit TypeScript declarations into `dist/` |
| `rush build --ssr <entry>` | - | - | Build client assets and a Node server bundle |
| `--port <port>` | `-p` | `3000` | Port for the HTTP and WebSocket server |
| `--root <dir>` | `-r` | `.` | Root directory containing static files and source code |
| `--open` | `-o` | - | Auto-open browser on server start |
| `--https` | - | - | Enable HTTPS with on-the-fly self-signed TLS certificates |
| `--proxy <prefix>=<url>` | - | - | Forward requests matching prefix to target URL |
| `--help` | `-h` | - | Display help information |

### Production Build Example

```bash
# Bundle, minify, hash assets, and output to dist/
rush build --root ./example
# Serve only the production output
python3 -m http.server 8080 --directory ./example/dist
```

Rush builds every `.html` file directly in the project root and keeps its filename in `dist/`.
For each page, it bundles the first local `<script type="module" src="...">` and rewrites the
page to reference its hashed bundle. Pages without a module script are copied with their
linked stylesheets; at least one page must have a module entry. Use links such as
`./about.html` to navigate between built pages.

Dynamic `import()` loads separately hashed JavaScript chunks on demand, and modules used
by multiple entries can be shared between pages. The build report lists the raw and
gzip-compressed size of each output file in KiB; it does not write `.gz` files.
In the example, click **Load lazy chunk** to see the filename loaded by the browser, or
follow **About this build** to open the second page and its own JavaScript entry.

### Vue, Svelte and Solid

Framework projects use Node.js and the compiler packages installed in the project's
`node_modules`. Install `esbuild` in each project, plus the packages below:

| Framework | Runtime dependencies | Development dependencies (besides `esbuild`) | Source files |
| --- | --- | --- | --- |
| Vue 3 | `vue` | `@vue/compiler-sfc` (match your Vue version) | `.vue` with templates, scripts or script setup (including TypeScript), and plain or scoped styles |
| Svelte 5 | `svelte` | — | `.svelte` with plain styles and types-only TypeScript |
| Solid | `solid-js` | `@babel/core`, `babel-preset-solid`, `solid-refresh` | `.jsx` and `.tsx` in a project declaring `solid-js` in `package.json` |

Import components with their file extensions. The dev server applies Vue template edits
without resetting local state; changes to a component's logic can remount it. Svelte's
compiler HMR and Solid Refresh replace edited components without reloading the page, but
local state can reset when the component remounts. Changes to SFC styles are applied
without a page reload. Restart the dev server after adding a new bare dependency import
so Rush can pre-bundle it.

Try any of the three counters by replacing `vue` below with `svelte` or `solid`:

```bash
npm install --prefix example-vue
rush --root ./example-vue --port 5173
# Edit Counter.vue with the dev page open at http://localhost:5173.

rush build --root ./example-vue
python3 -m http.server 8080 --directory ./example-vue/dist
```

### Server-Side Rendering (SSR)

In development, run Rush for the browser assets and HMR, and use the Node helper
[`ssr.mjs`](ssr.mjs) in your HTTP server. SSR requires Node.js 20+ and
`esbuild` installed in the app. Entries are paths relative to the project root:

```js
import { createSsrLoader } from './ssr.mjs';

const loader = createSsrLoader({ root: process.cwd(), origin: 'http://127.0.0.1:5173' });
// In the request handler:
const { render } = await loader.ssrLoadModule('src/entry-server.tsx');
const markup = await render();
// At server shutdown: await loader.close();
```

The helper runs the `rush` executable on PATH (override it with the `rush`
option). Use the same project root for the helper and the dev server. Rush uses
its esbuild pipeline with a Node target, TypeScript/JSX and SSR-aware plugin
hooks; browser-only HMR wrappers and CSS side effects don't run on the server.
CSS Module class names are available in both environments. The loader caches
the evaluated module until Rush's watcher detects a project edit, including
transitive dependencies. Call `ssrLoadModule` in each request rather than
keeping the initial module at startup. Installed packages remain external and
are loaded by Node. As with any local dev server, only run against trusted
projects; never use this loader in production.

Serve the browser scripts and `/__hmr.js` from Rush on the page's origin to
keep client updates working. The example uses Rush's HTTP proxy to forward
only the page route to Node; Node sends the rendered markup and the browser
entry calls React's `hydrateRoot`.

`rush build --ssr <entry>` runs the normal HTML/client build and adds
`dist/server/entry.mjs` and `dist/ssr-manifest.json`. Import the server entry
from Node and keep the app's runtime dependencies installed. The manifest maps
each HTML page with a module script to `{ "script": "./assets/…", "css": [...] }`,
with URLs relative to the client output root. Use it to add the client script
and styles to your rendered HTML. Serve only client assets publicly, not
`dist/server/`.

Run the [React SSR example](example-ssr/) from the repository root:

```sh
cargo build
export PATH="$PWD/target/debug:$PATH"  # also set this in the second terminal
cd example-ssr
npm ci
npm run dev:rush                      # terminal 1
npm run dev                           # terminal 2, also in example-ssr
# Visit http://127.0.0.1:5173/ssr/ and click the counter.

# Stop the dev servers before starting production.
npm run build
npm start
# Visit http://127.0.0.1:4173/ and click the counter again.
```

### Library Mode

`rush build --lib [--entry <path>] [--root <dir>]` uses the explicit entry (relative
to the root), the `source` field in `package.json`, or the first existing
`src/index.{ts,tsx,mts,js,jsx,mjs}`, in that order. It replaces the contents of
`dist/` with `<entry-stem>.mjs`, `<entry-stem>.cjs`, their source maps, and
declarations for a TypeScript entry. Runtime, peer, and optional dependencies
(including their subpaths) and `node:` imports remain external; imported
development dependencies are bundled.

Use explicit types for exported APIs where TypeScript's isolated declaration
rules require them. Keep relative TypeScript modules under the entry's
directory so their layout can be preserved in `dist/`. JavaScript entries
are bundled without declarations. Rush emits `.d.ts` files and matching
`.d.mts` and `.d.cts` files for the ESM and CommonJS export conditions.
See [the example's package.json](example-lib/package.json) for the export map;
Rush does not edit your manifest.

From the repository root, build the example and run both consumers:

```bash
npm ci --prefix example          # provides esbuild for this checkout
npm ci --prefix example-lib      # installs the example's local dependency
PATH="$PWD/example/node_modules/.bin:$PATH" cargo run -- build --lib --root example-lib
node example-lib/consumer/esm.mjs # Hello, ESM: 42
node example-lib/consumer/cjs.cjs # Hello, CJS: 42
npx --yes --package typescript tsc --noEmit --strict --module node16 \
  example-lib/consumer/check.ts example-lib/consumer/check.cts
```

### HTTPS Example

```bash
# Start local dev server with zero-config HTTPS
rush --root ./example --https
```

### Proxy Example

```bash
# Forward /api/* requests to a backend server
cargo run -- --root ./app --proxy /api=http://localhost:8080
```

---

## CSS Modules, PostCSS, and Tailwind

Rush serves regular stylesheets linked from HTML or imported for their side effects in JavaScript (`import './style.css'`). To scope classes to a file, use a `.module.css` extension and import its class map:

```js
import styles, { button } from './button.module.css';
element.classList.add(styles.button); // same as the named export button
```

Install Node.js and the following packages in the project for CSS Modules. Class names are scoped by file and are the same in development and production. Class names such as `primary-button` are available on the default map as `styles['primary-button']`.

```bash
npm install -D esbuild postcss postcss-modules
```

For a PostCSS project, install `postcss` and `postcss-load-config` alongside the project's plugins. Rush loads the root PostCSS config in both dev and `rush build` (`postcss.config.js`, `.cjs`, `.mjs`, `.postcssrc` and its supported extensions, or the `postcss` field in `package.json`). YAML and TypeScript config formats may require the optional loader dependencies documented by [postcss-load-config](https://github.com/postcss/postcss-load-config). Production builds with PostCSS or CSS Module imports use the project's installed `esbuild` package.

The example uses Tailwind v4. Install `tailwindcss` and `@tailwindcss/postcss`, add `'@tailwindcss/postcss': {}` to the PostCSS config's `plugins` object, and put `@import "tailwindcss";` in the global stylesheet. Automatic content detection works from the project root; add an `@source` directive to include files Tailwind would otherwise skip. In dev, Rush invalidates the PostCSS cache on project edits so Tailwind picks up added or removed classes. Tailwind v3 still works with its `tailwindcss` PostCSS plugin, explicit content globs, and `@tailwind` directives.

Stylesheet edits hot-swap the CSS without losing page state. If a CSS Module's exported classes change, Rush reloads the page so importers receive the new map.

## Vite and Rollup Plugins

Rush can run a subset of Vite and Rollup plugin hooks in dev and in the HTML app build (`rush build`, not `rush build --lib`). Add a config in your project root and install the plugins there:

```bash
npm install -D esbuild @rollup/plugin-virtual
```

```js
// rush.config.mjs
import virtual from '@rollup/plugin-virtual';

export default {
  plugins: [
    virtual({ 'virtual:greeting': 'export default "Hello from Rush";' }),
  ],
};
```

Import the generated module with `import greeting from 'virtual:greeting'`. Rush picks the first config it finds in this order: `rush.config.js`, `rush.config.mjs`, `rush.config.ts`, then the same extensions for `vite.config`. Export an object or a function (sync or async) receiving `{ command, mode }`. `command` is `serve` or `build`; `mode` is `development` or `production`. You don't need Vite unless your config or plugins import it. Rush loads the config with Node.js; use `.mjs` for unambiguous ESM, and Node 22.18+ with erasable TypeScript syntax for `.ts`. Builds using plugins need the project's `esbuild` npm package. Configs and plugins execute as trusted local JavaScript; only run configs and install plugins you trust.

| Hook or API | Rush support |
| --- | --- |
| Plugin selection and order | Nested arrays, promises, and falsy entries; Vite's `apply` string or predicate and `enforce: 'pre' / 'post'`; hook functions or `{ handler, order }`. Hook order takes precedence over plugin order. Hook filters and parallel/sequential hints are not implemented. |
| `config`, `configResolved` | Both modes. Returned config is merged for subsequent hooks. Vite's config fields don't reconfigure Rush (including aliases, root, build options, and server options), and plugins added from a `config` hook aren't loaded. |
| `resolveId`, `load`, `transform` | Both modes. Supports virtual IDs (including `\0` prefixes), external results, and code hooks returning a string or `{ code, map }`. In dev, Rush resolves static imports, re-exports and string-literal dynamic imports in app modules and serves registered virtual IDs under `/@id/`. Builds don't redirect HTML entry scripts through `resolveId`. Only JavaScript/TypeScript and common text inputs are read from disk for plugin transforms; a plugin can return module code from `load` for other imports. Build passes the final plugin map to esbuild when present; dev does not consume plugin maps, and Rush does not compose maps between plugins. |
| `buildStart`, `buildEnd` | `buildStart` runs in both modes. `buildEnd` runs after the production bundle, including when esbuild fails. It also runs when the dev sidecar receives EOF, but isn't guaranteed on server termination. |
| `configureServer` | Dev only. Use `server.middlewares.use([path,] handler)` before Rush routing. Handlers receive `req.url`, `originalUrl`, `method`, `headers`, and a buffered response with `statusCode`, header methods, `writeHead`, `write`, and `end`. Call `next()` to continue. No request body/stream, returned post-hook, real Node HTTP server, Vite WebSocket API, or Vite module graph. |
| Plugin context | `this.resolve` (with `skipSelf`, true by default), `this.warn`, `this.error`, and `this.meta.watchMode`. Build plugins can emit assets with `emitFile`, `getFileName`, and `setAssetSource`; emitting chunks or using `import.meta.ROLLUP_FILE_URL_*` isn't supported. |
| Other Vite/Rollup APIs | No `transformIndexHtml`, plugin-specific HMR (`handleHotUpdate`/`hotUpdate`), SSR, output hooks (such as `renderChunk` and `generateBundle`), or full Rollup context (`parse`, `addWatchFile`, `getModuleInfo`, etc.). Rush's built-in HTML, CSS, dependency pre-bundling, and HMR pipelines still apply. Install dependencies explicitly in projects with plugin configs and restart dev after changing the config or plugin code. |

The [example config](example/rush.config.mjs) uses three published Rollup plugins: `@rollup/plugin-virtual` supplies a greeting, `@rollup/plugin-yaml` imports a YAML object, and `@rollup/plugin-replace` inserts the current mode. The example page displays all three results in dev and after a production build.

## Example Project

The included `example/` folder demonstrates Rush serving HTML, CSS, JavaScript, and TypeScript together:

```
example/
├── index.html     # HTML entry point (Rush injects client script)
├── style.css      # Global stylesheet with Tailwind utilities
├── card.module.css # Scoped styles imported from JavaScript
├── postcss.config.cjs # PostCSS and Tailwind configuration
├── main.js        # JavaScript application code
├── counter.ts     # TypeScript file with import.meta.hot.accept()
├── config.json    # JSON module imported directly into JS
├── tsconfig.json  # Path aliases (@/* -> ./*)
└── .env           # Environment variables (RUSH_-prefixed)
```

Start the demo server:

```bash
npm ci --prefix example
cargo run -- --root example --port 3000 --open
```

Then open `http://localhost:3000` in your browser. Any edit to `style.css` will hot-swap instantly; edits to `.ts` or `.html` will trigger a clean reload. Syntax errors show a red overlay in the browser.

### Single Page Application (SPA) Routing Fallback

Rush automatically supports HTML5 History API routing. Any navigation request to paths without a file extension (e.g. `/dashboard` or `/settings`) automatically falls back to `index.html` with HMR injected.

### React Fast Refresh

React JSX/TSX components automatically benefit from React Fast Refresh transforms via `oxc` and runtime hooks, allowing components to hot-reload while preserving state.

### Auto-Install Dependencies

If a bare import cannot be resolved, Rush automatically detects your package manager (`pnpm`, `yarn`, `bun`, or `npm`), installs the missing package, and pre-bundles it without leaving your workflow.

---

## Architecture & Design Philosophy

Rush is built on **radical pragmatism and first-principles simplicity**: delivering a complete, modern developer experience by cutting away accidental complexity, avoiding dependency bloat, and embracing native platform features.

1. **Native Platform Capabilities First**: Modern browsers natively support ES modules (`<script type="module">`). Dev mode serves native ESM directly over HTTP instead of burning cycles re-bundling the entire project on every keystroke. Terminal styling uses direct ANSI escape sequences rather than adding terminal-color crates.
2. **Standard Library Over Dependency Creep**: When a task can be achieved cleanly with Rust's standard library, we resist pulling in extra crates. CLI argument parsing uses `std::env::args`, script injection uses straightforward string operations, cache verification uses `DefaultHasher`, and `.env` parsing is a concise string parser that enforces secure `RUSH_` scoping.
3. **Deep Tooling Synergy**: Rather than stitching together fragmented or conflicting compilers, Rush uses unified `oxc` capabilities (`transformer`, `semantic`, `codegen`) to handle parsing, TypeScript type stripping, JSX lowering, and React Fast Refresh in a single microsecond-level pass.
4. **Deliberate Pragmatism (YAGNI)**: We avoid speculative abstractions and heavyweight machinery in favor of direct, observable solutions:
   - High-performance directory filtering (`node_modules`, `.git`, `target`) without the overhead of heavy `.gitignore` parsers.
   - Immediate error overlay pushed via lightweight WebSockets rather than dragging in bloated error-runtime dependencies.
   - Microsecond on-the-fly AST transformations eliminate the need for complicated, invalidation-prone in-memory file caching layers.

---

## Testing

```bash
npm ci --prefix example   # Node dependencies used by build, CSS, framework, and plugin integration tests
cargo test
```

- **Unit tests** (in `src/`): transpilation and source maps, bare-import rewriting and package entry resolution, path aliases, `.env` loading, and the HMR module graph (boundaries, cycles, stale edges).
- **Integration tests** (in `tests/`): start a real `rush` process on a temp project and check served output, SPA fallback, JSON/asset modules, `--proxy`, the HMR messages sent over `/__hmr` when files change, and `rush build`.

## Benchmarks

See the [reproducible cold-start and HMR benchmarks](bench/README.md) for Rush, Vite, and Rspack, including the methodology and sample results.

## Roadmap

- [x] **Phase 1**: On-the-fly TypeScript / JSX / TSX Transpilation (`oxc`)
- [x] **Phase 2**: Bare Import Resolution (rewrite `import React from 'react'` for browsers)
- [x] **Phase 3**: Dependency Pre-bundling (bundle `node_modules` dependencies into ESM via `esbuild`)
- [x] **Phase 4**: Granular `import.meta.hot` module-level HMR
- [x] **Phase 5**: JSON & Static Asset imports, Source Maps
- [x] **Phase 6**: `--open` flag (auto-open browser on startup)
- [x] **Phase 7**: Error overlay (browser-side transpile error display)
- [x] **Phase 8**: Colored CLI output (ANSI escape codes)
- [x] **Phase 9**: `.env` file loading + `import.meta.env` replacement
- [x] **Phase 10**: `--proxy` API request forwarding
- [x] **Phase 11**: Path aliases from `tsconfig.json`
- [x] **Phase 12**: SPA routing fallback (HTML5 History API)
- [x] **Phase 13**: HTTPS dev server (`--https` with on-the-fly self-signed certs)
- [x] **Phase 14**: Auto-install missing dependencies (`pnpm`, `yarn`, `bun`, `npm`)
- [x] **Phase 15**: React Fast Refresh (state-preserving HMR)
- [x] **Phase 16**: Production build (`rush build` into `dist/`)

---

## License

MIT
