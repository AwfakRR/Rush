<p align="center">
  <img src="assets/logo.png" alt="Rush Logo" width="150" />
</p>

<h1 align="center">Rush</h1>

<p align="center">
  <b>A blazingly fast, minimalist dev server written in Rust.</b><br/>
  Built with the <i>Ponytail philosophy</i> (the simplest, shortest, most efficient solution that works).
</p>

<p align="center">
  <a href="#features">Features</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#cli-options">CLI Options</a> •
  <a href="#example-project">Example</a> •
  <a href="#architecture--ponytail-engineering">Architecture</a> •
  <a href="#roadmap">Roadmap</a> •
  <a href="#license">License</a>
</p>

---

Rush provides instant local web development with on-the-fly TypeScript/JSX transpilation, debounced file watching, WebSocket-based live reload, and CSS hot swapping — all without the overhead of heavy JavaScript runtimes or multi-megabyte dependency trees.

## Features

- **⚡ Sub-millisecond Transpilation**: Instant on-the-fly TypeScript (`.ts`), TSX (`.tsx`), and JSX (`.jsx`) transpilation powered by [`oxc`](https://github.com/oxc-project/oxc) — the high-performance Rust toolchain behind Rolldown.
- **🔄 Live Reload & CSS Hot Swap**: WebSocket connection automatically reloads the browser on file saves. Changes to `.css` files hot-swap stylesheets instantly without losing page state or triggering a full refresh.
- **🔥 Granular Module HMR**: Supports `import.meta.hot.accept()` and dependency graph boundary resolution to update individual JS/TS modules in-place.
- **📦 Bare Import Resolution & Pre-bundling**: Rewrites bare npm specifiers (`import React from 'react'`) and automatically pre-bundles CommonJS/ESM dependencies with `esbuild`.
- **🗺️ Standard v3 Source Maps**: Generates and serves in-memory source maps (`.map`) for seamless in-browser debugging of original TypeScript source.
- **📄 JSON & Static Asset Modules**: Import JSON files directly as default objects, and static media files (`.png`, `.svg`, etc.) as URL strings.
- **🛡️ Debounced File Watching**: 50ms debouncer coalesces IDE and editor atomic saves into single clean reload triggers.
- **💉 Zero-Config HTML Injection**: Automatically injects the lightweight client runtime into `<head>` before serving HTML documents.
- **✂️ Minimal Dependency Footprint**: Built with just 5 core Rust crates, prioritizing standard library APIs and native platform features over dependency bloat.

---

## Quick Start

### Prerequisites
- [Rust](https://rustup.rs/) (1.96.0+ recommended)

### Build & Run

Clone the repository and run:

```bash
# Serve the current directory on default port 3000
cargo run

# Serve a specific folder on a custom port
cargo run -- --root ./example --port 5173
```

Build an optimized release binary:

```bash
cargo build --release
./target/release/rush --root ./public --port 8080
```

---

## CLI Options

| Flag | Short | Default | Description |
|---|---|---|---|
| `--port <port>` | `-p` | `3000` | Port for the HTTP and WebSocket server |
| `--root <dir>` | `-r` | `.` | Root directory containing static files and source code |
| `--open` | `-o` | - | Auto-open browser on server start |
| `--proxy <prefix>=<url>` | - | - | Forward requests matching prefix to target URL |
| `--help` | `-h` | - | Display help information |

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
└── .env           # Environment variables (RUSH_-prefixed)
```

Start the demo server:

```bash
cargo run -- --root example --port 3000 --open
```

Then open `http://localhost:3000` in your browser. Any edit to `style.css` will hot-swap instantly; edits to `.ts` or `.html` will trigger a clean reload. Syntax errors show a red overlay in the browser.

### Environment Variables

Create a `.env` file in your project root:

```env
RUSH_API_URL=http://localhost:8080
RUSH_APP_NAME=My App
```

Access in code via `import.meta.env.RUSH_API_URL`. Built-in vars: `import.meta.env.DEV` (`true`), `import.meta.env.MODE` (`"development"`).

### Path Aliases

Add a `tsconfig.json` with `compilerOptions.paths`:

```json
{
  "compilerOptions": {
    "paths": { "@/*": ["./src/*"] }
  }
}
```

Then `import { Foo } from '@/components/Foo'` resolves to `./src/components/Foo`.

---

## Architecture & Ponytail Engineering

Rush follows strict **Ponytail engineering principles**:

1. **Native Platform Capabilities First**: Modern browsers natively support ES modules (`<script type="module">`). Dev mode serves native ESM instead of doing unnecessary full-project bundling on every keystroke.
2. **Standard Library Over Dependencies**: CLI parsing uses `std::env::args`, HTML script injection uses standard `String::find`, cache hashing uses `DefaultHasher`, colored output uses raw ANSI codes, and `.env` parsing is hand-rolled string splitting.
3. **Sub-crate Lockstep**: Uses unified `oxc` features (`transformer`, `semantic`, `codegen`) to guarantee compatibility across AST, parser, and code generation steps.
4. **Deliberate Simplifications**: Every shortcut is documented with clear boundaries:
   - Ignored noise directories (`node_modules`, `.git`, `target`) without heavy gitignore parser overhead.
   - Transpile errors shown as a browser overlay with WebSocket push.
   - Sub-millisecond AST transformations eliminate the need for complex in-memory file caching.

---

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

---

## License

MIT
