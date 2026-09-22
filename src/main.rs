use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Request, State,
    },
    http::header,
    middleware::{from_fn_with_state, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::sync::broadcast;
use tower_http::services::ServeDir;

mod transform;
mod resolve;
mod hmr;

#[derive(Clone)]
struct AppState {
    root: PathBuf,
    tx: broadcast::Sender<String>,
    graph: Arc<RwLock<hmr::ModuleGraph>>,
    source_maps: Arc<RwLock<HashMap<String, String>>>,
}

const HMR_CLIENT: &str = r#"(() => {
  const socket = new WebSocket(`ws://${location.host}/__hmr`);
  socket.addEventListener('open', () => console.log('[rush] connected'));
  socket.addEventListener('message', ({ data }) => {
    let msg;
    try {
      msg = JSON.parse(data);
    } catch {
      msg = { type: data.endsWith('.css') ? 'css' : 'full-reload', path: data };
    }

    if (msg.type === 'css') {
      console.log('[rush] css update:', msg.path);
      const links = document.querySelectorAll('link[rel="stylesheet"]');
      for (const link of links) {
        const url = new URL(link.href);
        url.searchParams.set('t', Date.now());
        link.href = url.href;
      }
    } else if (msg.type === 'js-update') {
      console.log('[rush] hot update:', msg.path);
      const ctx = window.__rush_hot?.get(msg.path);
      if (ctx?._disposeCallbacks) {
        for (const cb of ctx._disposeCallbacks) {
          try { cb(ctx.data); } catch (e) { console.error(e); }
        }
        ctx._disposeCallbacks = [];
      }
      import(`${msg.path}?t=${msg.timestamp}`).then(newMod => {
        if (ctx?._acceptCallbacks) {
          for (const cb of ctx._acceptCallbacks) {
            try { cb(newMod); } catch (e) { console.error(e); }
          }
        }
      }).catch(err => {
        console.error('[rush] hot update failed, reloading:', err);
        location.reload();
      });
    } else {
      console.log('[rush] full reload');
      location.reload();
    }
  });
  socket.addEventListener('close', () => {
    console.log('[rush] disconnected, reconnecting...');
    setTimeout(() => location.reload(), 1500);
  });
})();"#;

fn inject_hmr(html: &str) -> String {
    let script = r#"<script type="module" src="/__hmr.js"></script>"#;
    // ponytail: simple string search, no HTML parser needed
    if let Some(pos) = html.find("</head>") {
        let (before, after) = html.split_at(pos);
        format!("{before}{script}\n{after}")
    } else if let Some(pos) = html.find("</body>") {
        let (before, after) = html.split_at(pos);
        format!("{before}{script}\n{after}")
    } else {
        format!("{script}\n{html}")
    }
}

async fn hmr_client() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/javascript")], HMR_CLIENT)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state.tx.subscribe()))
}

async fn handle_ws(mut socket: WebSocket, mut rx: broadcast::Receiver<String>) {
    while let Ok(msg) = rx.recv().await {
        if socket.send(Message::Text(msg.into())).await.is_err() {
            break;
        }
    }
}

async fn request_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    let rel_path = path.trim_start_matches('/');
    let target = if rel_path.is_empty() {
        state.root.join("index.html")
    } else {
        state.root.join(rel_path)
    };
    let target = if target.is_dir() {
        target.join("index.html")
    } else {
        target
    };

    let ext = target.extension().and_then(|e| e.to_str()).unwrap_or("");

    // Phase 5: Serve in-memory source map
    if path.ends_with(".map") {
        let map_key = hmr::normalize_path(rel_path);
        if let Some(map_json) = state.source_maps.read().unwrap().get(&map_key).cloned() {
            return ([(header::CONTENT_TYPE, "application/json")], map_json).into_response();
        }
    }

    // HTML: inject HMR client script
    if target.is_file() && ext == "html" {
        if let Ok(content) = tokio::fs::read_to_string(&target).await {
            return Html(inject_hmr(&content)).into_response();
        }
    }

    // Phase 5: JSON modules: export default { ... }
    // ponytail: one format!() wrap. No JSON re-serialization needed.
    if target.is_file() && ext == "json" {
        if let Ok(content) = tokio::fs::read_to_string(&target).await {
            let code = format!("export default {};\n", content.trim());
            return ([(header::CONTENT_TYPE, "application/javascript")], code).into_response();
        }
    }

    // Phase 5: Static assets imported as JS module
    const ASSET_EXTS: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "svg", "webp", "ico",
        "woff", "woff2", "ttf", "eot",
        "mp4", "webm", "ogg", "mp3", "wav",
        "pdf",
    ];
    if target.is_file() && ASSET_EXTS.contains(&ext) {
        let is_module_import = req.headers().get("sec-fetch-dest").and_then(|v| v.to_str().ok()) == Some("script")
            || req.uri().query().map(|q| q.contains("import")).unwrap_or(false);

        if is_module_import {
            let url = hmr::normalize_path(rel_path);
            let code = format!("export default \"{url}\";\n");
            return ([(header::CONTENT_TYPE, "application/javascript")], code).into_response();
        }
    }

    // Helper to register module dependencies into ModuleGraph
    let register_module = |source: &str, fname: &str| {
        let is_accepting = hmr::has_hot_accept(source);
        let raw_imports = resolve::scan_imports(source, fname);
        let parent = Path::new(rel_path).parent().unwrap_or(Path::new(""));
        let mut deps = Vec::new();
        for (spec, _, _) in raw_imports {
            if spec.starts_with('.') {
                let resolved = parent.join(spec);
                deps.push(hmr::normalize_path(&resolved.to_string_lossy()));
            }
        }
        let mod_path = hmr::normalize_path(rel_path);
        state.graph.write().unwrap().update(&mod_path, &deps, is_accepting);
    };

    // TS/TSX/JSX: transpile on the fly via oxc
    // ponytail: no caching of transpile results — files are small, oxc is microseconds.
    // Add in-memory cache if profiler says transpilation is the bottleneck.
    if target.is_file() && matches!(ext, "ts" | "tsx" | "jsx") {
        if let Ok(source) = tokio::fs::read_to_string(&target).await {
            let fname = target.file_name().unwrap().to_string_lossy().to_string();
            register_module(&source, &fname);
            match transform::transpile(&fname, &source) {
                Ok((code, _map)) => {
                    // Phase 2: Rewrite bare imports before serving to browser
                    let rewritten = resolve::rewrite_bare_imports(&code, &fname, "/node_modules/.rush/deps");
                    // Phase 4: Inject import.meta.hot preamble
                    let mod_path = hmr::normalize_path(rel_path);
                    let mut with_hmr = hmr::inject_hot_preamble(&rewritten, &mod_path);
                    // Phase 5: Cache source map and append sourceMappingURL
                    if let Some(map_json) = _map {
                        let map_path = format!("{mod_path}.map");
                        state.source_maps.write().unwrap().insert(map_path.clone(), map_json);
                        with_hmr.push_str(&format!("\n//# sourceMappingURL={map_path}\n"));
                    }
                    return ([(header::CONTENT_TYPE, "application/javascript")], with_hmr)
                        .into_response();
                }
                Err(e) => {
                    // ponytail: show error in browser console, not a custom overlay
                    eprintln!("[rush] transpile error: {e}");
                    let escaped = e.replace('\\', "\\\\").replace('`', "\\`");
                    return (
                        [(header::CONTENT_TYPE, "application/javascript")],
                        format!("console.error(`[rush] {escaped}`);"),
                    )
                        .into_response();
                }
            }
        }
    }

    // JS: rewrite bare imports for browser execution
    if target.is_file() && ext == "js" {
        if let Ok(source) = tokio::fs::read_to_string(&target).await {
            let fname = target.file_name().unwrap().to_string_lossy().to_string();
            register_module(&source, &fname);
            let rewritten = resolve::rewrite_bare_imports(&source, &fname, "/node_modules/.rush/deps");
            // Phase 4: Inject import.meta.hot preamble
            let mod_path = hmr::normalize_path(rel_path);
            let with_hmr = hmr::inject_hot_preamble(&rewritten, &mod_path);
            return ([(header::CONTENT_TYPE, "application/javascript")], with_hmr)
                .into_response();
        }
    }

    next.run(req).await
}

fn parse_args() -> (u16, PathBuf) {
    // ponytail: std::env::args covers this, clap is 50 crates for two flags
    let mut port = 3000;
    let mut root = PathBuf::from(".");
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" if i + 1 < args.len() => {
                port = args[i + 1].parse().unwrap_or(3000);
                i += 1;
            }
            "--root" | "-r" if i + 1 < args.len() => {
                root = PathBuf::from(&args[i + 1]);
                i += 1;
            }
            "--help" | "-h" => {
                println!("Usage: rush [--port <port>] [--root <dir>]");
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }
    (port, root)
}

fn should_ignore(path: &Path) -> bool {
    // ponytail: ignore common noise directories without a full .gitignore parser
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s == "node_modules" || s == ".git" || s == "target"
    })
}

#[tokio::main]
async fn main() {
    let (port, root) = parse_args();
    let root = match root.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: Cannot resolve root directory {}: {}", root.display(), e);
            std::process::exit(1);
        }
    };

    // Phase 3: Pre-bundle node_modules dependencies via esbuild
    if let Err(e) = resolve::prebundle(&root) {
        eprintln!("[rush] prebundle notice: {e}");
    }

    let (tx, _rx) = broadcast::channel::<String>(16);
    let graph = Arc::new(RwLock::new(hmr::ModuleGraph::new()));

    // Spawn debounced file watcher in background thread
    let watcher_root = root.clone();
    let watcher_tx = tx.clone();
    let watcher_graph = graph.clone();
    std::thread::spawn(move || {
        let (sync_tx, sync_rx) = std::sync::mpsc::channel();
        let mut debouncer = match new_debouncer(Duration::from_millis(50), sync_tx) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[rush] watcher error: {}", e);
                return;
            }
        };

        if let Err(e) = debouncer.watcher().watch(&watcher_root, RecursiveMode::Recursive) {
            eprintln!("[rush] watcher register error: {}", e);
            return;
        }

        while let Ok(events) = sync_rx.recv() {
            if let Ok(events) = events {
                for event in events {
                    if should_ignore(&event.path) {
                        continue;
                    }
                    let rel = event
                        .path
                        .strip_prefix(&watcher_root)
                        .unwrap_or(&event.path)
                        .to_string_lossy()
                        .replace('\\', "/");

                    let ext = event.path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();

                    if ext == "css" {
                        println!("[rush] css update: {}", rel);
                        let msg = serde_json::json!({
                            "type": "css",
                            "path": hmr::normalize_path(&rel),
                            "timestamp": now,
                        });
                        watcher_tx.send(msg.to_string()).ok();
                    } else if matches!(ext, "js" | "ts" | "jsx" | "tsx") {
                        let boundaries = watcher_graph.read().unwrap().find_update_boundaries(&rel);
                        if let Some(targets) = boundaries {
                            println!("[rush] hot update: {:?}", targets);
                            for target in targets {
                                let msg = serde_json::json!({
                                    "type": "js-update",
                                    "path": target,
                                    "timestamp": now,
                                });
                                watcher_tx.send(msg.to_string()).ok();
                            }
                        } else {
                            println!("[rush] reload: {}", rel);
                            let msg = serde_json::json!({
                                "type": "full-reload",
                                "path": hmr::normalize_path(&rel),
                            });
                            watcher_tx.send(msg.to_string()).ok();
                        }
                    } else {
                        println!("[rush] reload: {}", rel);
                        let msg = serde_json::json!({
                            "type": "full-reload",
                            "path": hmr::normalize_path(&rel),
                        });
                        watcher_tx.send(msg.to_string()).ok();
                    }
                }
            }
        }
    });

    let source_maps = Arc::new(RwLock::new(HashMap::new()));

    let state = AppState {
        root: root.clone(),
        tx,
        graph,
        source_maps,
    };

    let app = Router::new()
        .route("/__hmr", get(ws_handler))
        .route("/__hmr.js", get(hmr_client))
        .fallback_service(ServeDir::new(&state.root))
        .layer(from_fn_with_state(state.clone(), request_middleware))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Error: Failed to bind to {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    println!();
    println!("  🚀 Rush dev server running at:");
    println!("  > Local:   http://localhost:{}/", port);
    println!("  > Network: http://127.0.0.1:{}/", port);
    println!("  > Root:    {}", root.display());
    println!();

    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inject_hmr() {
        let html = "<html><head><title>Test</title></head><body><h1>Hi</h1></body></html>";
        let injected = inject_hmr(html);
        assert!(injected.contains(r#"<script type="module" src="/__hmr.js"></script>"#));
        assert!(injected.find(r#"<script type="module" src="/__hmr.js"></script>"#).unwrap() < injected.find("</head>").unwrap());

        let no_head = "<html><body><h1>Hi</h1></body></html>";
        let injected_no_head = inject_hmr(no_head);
        assert!(injected_no_head.contains(r#"<script type="module" src="/__hmr.js"></script>"#));
    }

    #[test]
    fn test_should_ignore() {
        assert!(should_ignore(Path::new("node_modules/foo/bar.js")));
        assert!(should_ignore(Path::new(".git/HEAD")));
        assert!(should_ignore(Path::new("target/debug/rush.exe")));
        assert!(!should_ignore(Path::new("src/main.rs")));
        assert!(!should_ignore(Path::new("index.html")));
    }
}
