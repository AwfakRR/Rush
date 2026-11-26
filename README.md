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

## Example Project

The included `example/` folder demonstrates Rush serving HTML, CSS, JavaScript, and TypeScript together:

```
example/
├── index.html     # HTML entry point (Rush injects client script)
├── style.css      # Stylesheet (hot-swapped on save)
├── main.js        # JavaScript application code
├── counter.ts     # TypeScript file with import.meta.hot.accept()
├── config.json    # JSON module imported directly into JS
├── tsconfig.json  # Path aliases (@/* -> ./*)
└── .env           # Environment variables (RUSH_-prefixed)
```

Start the demo server:

```bash
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
npm install --prefix example   # esbuild, used by the `rush build` tests
cargo test
```

- **Unit tests** (in `src/`): transpilation and source maps, bare-import rewriting and package entry resolution, path aliases, `.env` loading, and the HMR module graph (boundaries, cycles, stale edges).
- **Integration tests** (in `tests/`): start a real `rush` process on a temp project and check served output, SPA fallback, JSON/asset modules, `--proxy`, the HMR messages sent over `/__hmr` when files change, and `rush build`.

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
