# Dev server benchmarks

Compare Rush, Vite, and Rspack on the same generated React/TSX app. These are
development-server measurements, not production build timings.

## Reproduce

From the repository root on Linux, with Node.js 22 (22.12 or newer) and a
current stable Rust toolchain:

```sh
cd bench && npm ci && npx playwright install --with-deps chromium && npm run bench
```

If Chromium's OS dependencies are already installed, use
`npx playwright install chromium` instead of the `--with-deps` command.
The runner builds Rush in release mode before timing and pins the JavaScript
dependencies and Chromium through `package-lock.json`. The default is five
runs per tool with 1,000 components. To change the size or repetitions:

```sh
npm run bench -- --components 250 --runs 10
```

To also use the browser benchmark as a regression test, run
`npm run bench -- --assert-rush-refresh`. It fails if a Rush root or leaf
edit reloads the page or loses React state in any trial.
It also checks, outside the timed edits, that a syntax error keeps the app
mounted behind an overlay, a fix preserves state, and an initially broken
page mounts after one reload.

The Markdown table goes to stdout; individual samples, summaries, machine
information, and tool versions go to `bench/results/<timestamp>.json`. The
manual **Benchmarks** GitHub Actions workflow uploads the same JSON as the
`benchmark-results` artifact.

## Method

- **Fixture:** One TSX module per component in a deterministic binary tree.
  The root and the last leaf render separate text markers. All tools use React
  17 and classic JSX so the app uses the same package entry points and source
  files for Rush, Vite, and Rspack, with no tool-specific HMR code in the app.
  Vite and Rspack use their React Fast Refresh plugins.
- **Cold trial:** A new server process and Chromium context for each sample,
  with the fixture's Rush, Vite, or Rspack disk cache removed before launching
  that tool. The order of the tools rotates by round. Dependency installation,
  the Rush build, fixture generation, and browser launch are untimed. OS
  filesystem caches are not flushed.
- **HTTP ready:** Server spawn to the first successful HTTP response for
  the fixture's `/`. The probe checks the returned HTML; this is not a port
  check. A server that compiles before serving HTML includes that work here.
- **Cold page:** Server spawn to Chromium observing both original markers and
  every leaf in the DOM. Browser navigation starts after the HTTP probe.
- **Root and leaf edits:** After the HMR WebSocket opens, the runner increments
  a React counter, edits the marker in the corresponding TSX file, and times
  from before the write until the changed marker and all leaves are in the DOM.
  The root is edited first, then the leaf. The runner counts main-frame
  navigations and checks whether the counter survived each edit.

Times are median (min–max) in milliseconds. Edit columns also show the number
of trials that reloaded and the number that preserved React state. A full
reload is not equivalent to an in-place HMR update.

## Sample results

Five runs per tool with 1,000 components and 500 leaves on 2026-11-26 UTC.
Machine: Ubuntu 22.04.5 x86-64 (Linux 6.8.0-1060-aws), Intel Xeon, 8 logical
CPUs, 31 GiB RAM. Versions: Rush 0.1.0 (`c4c464b`), Vite 8.3.8,
Rspack 2.3.4, Node.js 22.23.5, Rust 1.99.1, Chromium 159.0.8154.6.

| Tool | HTTP ready (ms) | Cold page (ms) | Root edit (ms) | Leaf edit (ms) |
| --- | ---: | ---: | ---: | ---: |
| Rush | 78.5 (77.0–269.6) | 513.2 (490.3–794.4) | 429.2 (336.4–552.7); 5/5 reloads; 0/5 state kept | 449.1 (322.6–537.9); 5/5 reloads; 0/5 state kept |
| Vite | 240.4 (231.3–264.6) | 1208.8 (1114.0–1264.2) | 79.7 (77.9–140.0); 0/5 reloads; 5/5 state kept | 64.8 (58.2–73.9); 0/5 reloads; 5/5 state kept |
| Rspack | 216.5 (203.3–331.3) | 395.7 (372.9–579.9) | 81.5 (78.4–107.2); 0/5 reloads; 5/5 state kept | 87.6 (85.4–94.0); 0/5 reloads; 5/5 state kept |

On this fixture, Rush reached HTTP readiness first, while Rspack rendered the
full cold page first. Vite and Rspack preserved state on both edits; Rush fell
back to full reload on both. The Rush edit times therefore represent
edit-to-render latency through a reload, not state-preserving Fast Refresh.
Results are specific to this fixture and machine.
