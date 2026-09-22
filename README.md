# Rush ⚡

A blazingly fast, minimalist dev server written in Rust, built with the **Ponytail philosophy** (the simplest, shortest, most efficient solution that works).

Rush provides instant local web development with on-the-fly TypeScript/JSX transpilation, debounced file watching, WebSocket-based live reload, and CSS hot swapping — all without the overhead of heavy JavaScript runtimes or multi-megabyte dependency trees.

---

## Features

- **⚡ Sub-millisecond Transpilation**: Instant on-the-fly TypeScript (`.ts`), TSX (`.tsx`), and JSX (`.jsx`) transpilation powered by [`oxc`](https://github.com/oxc-project/oxc) — the high-performance Rust toolchain behind Rolldown.
- **🔄 Live Reload & CSS Hot Swap**: WebSocket connection automatically reloads the browser on file saves. Changes to `.css` files hot-swap stylesheets instantly without losing page state or triggering a full refresh.
- **🛡️ Debounced File Watching**: 50ms debouncer coalesces IDE and editor atomic saves into single clean reload triggers.
- **💉 Zero-Config HTML Injection**: Automatically injects the lightweight client runtime into `<head>` before serving HTML documents.
- **📦 High-Performance Static Serving**: Async HTTP file serving with streaming and range support backed by `axum` and `tower-http`.
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
| `--help` | `-h` | - | Display help information |

---

## Example Project

The included `example/` folder demonstrates Rush serving HTML, CSS, JavaScript, and TypeScript together:

```
example/
├── index.html     # HTML entry point (Rush injects client script)
├── style.css      # Stylesheet (hot-swapped on save)
├── main.js        # JavaScript application code
└── counter.ts     # TypeScript file transpiled on-the-fly to JS
```

Start the demo server:

```bash
cargo run -- --root example --port 3000
```

Then open `http://localhost:3000` in your browser. Any edit to `style.css` will hot-swap instantly; edits to `.ts` or `.html` will trigger a clean reload.

---

## Architecture & Ponytail Engineering

Rush follows strict **Ponytail engineering principles**:

1. **Native Platform Capabilities First**: Modern browsers natively support ES modules (`<script type="module">`). Dev mode serves native ESM instead of doing unnecessary full-project bundling on every keystroke.
2. **Standard Library Over Dependencies**: CLI parsing uses `std::env::args`, HTML script injection uses standard `String::find`, and cache hashing uses `DefaultHasher`.
3. **Sub-crate Lockstep**: Uses unified `oxc` features (`transformer`, `semantic`, `codegen`) to guarantee compatibility across AST, parser, and code generation steps.
4. **Deliberate Simplifications**: Every shortcut is documented with clear boundaries:
   - Ignored noise directories (`node_modules`, `.git`, `target`) without heavy gitignore parser overhead.
   - Transpile errors are reported directly to the browser console.
   - Sub-millisecond AST transformations eliminate the need for complex in-memory file caching.

---

## Roadmap

- [x] **Phase 1**: On-the-fly TypeScript / JSX / TSX Transpilation (`oxc`)
- [x] **Phase 2**: Bare Import Resolution (rewrite `import React from 'react'` for browsers)
- [x] **Phase 3**: Dependency Pre-bundling (bundle `node_modules` dependencies into ESM via `esbuild`)
- [ ] **Phase 4**: Granular `import.meta.hot` module-level HMR
- [ ] **Phase 5**: JSON & Static Asset imports, Source Maps

---

## License

MIT
