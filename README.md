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
npm ci --prefix example   # Node dependencies used by the build and CSS integration tests
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
