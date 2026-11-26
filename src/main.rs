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
use axum_server::tls_rustls::RustlsConfig;
use notify_debouncer_mini::{new_debouncer_opt, notify, notify::RecursiveMode};
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
mod log;
mod env;
mod alias;
mod build;
#[cfg(test)]
mod test_util;

#[derive(Clone)]
struct AppState {
    root: PathBuf,
    tx: broadcast::Sender<String>,
    graph: Arc<RwLock<hmr::ModuleGraph>>,
    source_maps: Arc<RwLock<HashMap<String, String>>>,
    env_vars: Arc<HashMap<String, String>>,
    proxies: Arc<Vec<(String, String)>>,
    aliases: Arc<Vec<(String, String)>>,
}

const HMR_CLIENT: &str = r#"import refresh from '/@react-refresh';

(() => {
  // Phase 7: Error overlay helpers
  // ponytail: inline DOM creation, no framework, no shadow DOM. CSS scoped by id.
  function showOverlay(msg) {
    let el = document.getElementById('__rush_error');
    if (!el) {
      el = document.createElement('div');
      el.id = '__rush_error';
      el.style.cssText = 'position:fixed;inset:0;z-index:99999;background:rgba(0,0,0,0.85);display:flex;align-items:center;justify-content:center;font-family:monospace';
      el.innerHTML = '<div style="background:#1a1a2e;color:#fff;border:2px solid #e74c3c;border-radius:8px;padding:24px;max-width:80vw;max-height:80vh;overflow:auto;position:relative"><button style="position:absolute;top:8px;right:12px;background:none;border:none;color:#888;font-size:20px;cursor:pointer" onclick="this.parentElement.parentElement.remove()">×</button><h3 style="color:#e74c3c;margin:0 0 12px">🚨 Rush Build Error</h3><pre style="color:#ff6b6b;white-space:pre-wrap;word-break:break-word;margin:0;font-size:13px"></pre></div>';
      document.body.appendChild(el);
    }
    el.querySelector('pre').textContent = msg;
  }
  function clearOverlay() {
    document.getElementById('__rush_error')?.remove();
  }

  let refreshTimer;
  function canRefresh(previous, next) {
    const names = Object.keys(previous);
    return names.length > 0 && names.length === Object.keys(next).length &&
      names.every(name => refresh.isLikelyComponentType(previous[name]) &&
        refresh.isLikelyComponentType(next[name]));
  }

  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const socket = new WebSocket(`${proto}//${location.host}/__hmr`);
  socket.addEventListener('open', () => console.log('[rush] connected'));
  socket.addEventListener('message', ({ data }) => {
    let msg;
    try {
      msg = JSON.parse(data);
    } catch {
      msg = { type: data.endsWith('.css') ? 'css' : 'full-reload', path: data };
    }

    if (msg.type === 'error') {
      console.error('[rush]', msg.message);
      showOverlay(msg.file ? msg.file + '\n\n' + msg.message : msg.message);
    } else if (msg.type === 'css') {
      clearOverlay();
      console.log('[rush] css update:', msg.path);
      const links = document.querySelectorAll('link[rel="stylesheet"]');
      for (const link of links) {
        const url = new URL(link.href);
        url.searchParams.set('t', Date.now());
        link.href = url.href;
      }
    } else if (msg.type === 'js-update') {
      clearOverlay();
      console.log('[rush] hot update:', msg.path);
      const ctx = window.__rush_hot?.get(msg.path);
      const previousUrl = ctx?._refreshUrl;
      if (ctx?._disposeCallbacks) {
        for (const cb of ctx._disposeCallbacks) {
          try { cb(ctx.data); } catch (e) { console.error(e); }
        }
        ctx._disposeCallbacks = [];
      }
      const nextUrl = `${msg.path}?t=${msg.timestamp}`;
      import(nextUrl).then(async newMod => {
        if (previousUrl) {
          if (ctx._refreshUrl !== new URL(nextUrl, location.href).href ||
              !canRefresh(await import(previousUrl), newMod)) {
            location.reload();
            return;
          }
          clearTimeout(refreshTimer);
          refreshTimer = setTimeout(() => refresh.performReactRefresh(), 16);
        }
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
      clearOverlay();
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
    let script = r#"<script type="module">
import refresh from '/@react-refresh';
refresh.injectIntoGlobalHook(window);
window.$RefreshReg$ = () => {};
window.$RefreshSig$ = () => type => type;
</script>
<script type="module" src="/__hmr.js"></script>"#;
    // ponytail: simple string search, no HTML parser needed
    let lower = html.to_ascii_lowercase();
    let pos = ["<script", "</head>", "</body>"]
        .iter()
        .filter_map(|tag| lower.find(tag))
        .min()
        .unwrap_or(0);
    let (before, after) = html.split_at(pos);
    format!("{before}{script}\n{after}")
}

async fn hmr_client() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/javascript")], HMR_CLIENT)
}

async fn react_refresh_runtime() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("../assets/react-refresh.js"),
    )
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

    // Phase 10: Proxy forwarding — check before any file resolution
    // ponytail: simple prefix match + reqwest forward, no streaming, no websocket proxy
    for (prefix, target_url) in state.proxies.iter() {
        if path.starts_with(prefix) {
            let rest = &path[prefix.len()..];
            let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
            let url = format!("{}{}{}", target_url.trim_end_matches('/'), rest, query);
            let method = req.method().clone();

            let client = reqwest::Client::new();
            let proxy_req = client.request(method, &url);

            match proxy_req.send().await {
                Ok(resp) => {
                    let status = axum::http::StatusCode::from_u16(resp.status().as_u16())
                        .unwrap_or(axum::http::StatusCode::BAD_GATEWAY);
                    let body = resp.bytes().await.unwrap_or_default();
                    return (status, body).into_response();
                }
                Err(e) => {
                    log::error(&format!("proxy error: {e}"));
                    return (
                        axum::http::StatusCode::BAD_GATEWAY,
                        format!("Proxy error: {e}"),
                    ).into_response();
                }
            }
        }
    }

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
    let prepare_module = |code: &str, fname: &str, is_accepting| {
        let mod_path = hmr::normalize_path(rel_path);
        let aliased = alias::resolve_aliases(code, fname, &state.aliases);
        let imports = resolve::scan_imports(&aliased, fname);
        let deps: Vec<_> = imports
            .iter()
            .filter_map(|(spec, _, _)| hmr::local_import_path(&mod_path, spec))
            .collect();
        let mut graph = state.graph.write().unwrap();
        graph.update(&mod_path, &deps, is_accepting);
        let rewritten = resolve::rewrite_imports(&aliased, imports, |specifier| {
            if resolve::is_bare(specifier) {
                Some(resolve::dep_url("/node_modules/.rush/deps", specifier))
            } else {
                let path = hmr::local_import_path(&mod_path, specifier)?;
                graph.updated_import(specifier, &path)
            }
        });
        (rewritten, mod_path)
    };

    // TS/TSX/JSX: transpile on the fly via oxc
    // ponytail: no caching of transpile results — files are small, oxc is microseconds.
    // Add in-memory cache if profiler says transpilation is the bottleneck.
    if target.is_file() && matches!(ext, "ts" | "tsx" | "jsx") {
        if let Ok(source) = tokio::fs::read_to_string(&target).await {
            let fname = target.file_name().unwrap().to_string_lossy().to_string();
            match transform::transpile(&fname, &source) {
                Ok(transformed) => {
                    let explicit = hmr::has_hot_accept(&source);
                    let (mut rewritten, mod_path) = prepare_module(
                        &transformed.code,
                        &fname,
                        explicit || transformed.refresh_registration,
                    );
                    if transformed.refresh_registration || transformed.refresh_signature {
                        rewritten = hmr::inject_refresh_preamble(
                            &rewritten,
                            transformed.refresh_registration && !explicit,
                        );
                    }
                    let mut with_hmr = hmr::inject_hot_preamble(&rewritten, &mod_path);
                    // Phase 5: Cache source map and append sourceMappingURL
                    if let Some(map_json) = transformed.map {
                        let map_path = format!("{mod_path}.map");
                        state.source_maps.write().unwrap().insert(map_path.clone(), map_json);
                        with_hmr.push_str(&format!("\n//# sourceMappingURL={map_path}\n"));
                    }
                    // Phase 9: Replace import.meta.env.* references
                    let final_code = env::replace_env_vars(&with_hmr, &state.env_vars);
                    return ([(header::CONTENT_TYPE, "application/javascript")], final_code)
                        .into_response();
                }
                Err(e) => {
                    // Phase 7: broadcast error for overlay + console fallback
                    log::error(&format!("transpile error: {e}"));
                    let error_msg = serde_json::json!({
                        "type": "error",
                        "file": rel_path,
                        "message": e,
                    });
                    state.tx.send(error_msg.to_string()).ok();
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
            let (rewritten, mod_path) =
                prepare_module(&source, &fname, hmr::has_hot_accept(&source));
            let with_hmr = hmr::inject_hot_preamble(&rewritten, &mod_path);
            // Phase 9: Replace import.meta.env.* references
            let final_code = env::replace_env_vars(&with_hmr, &state.env_vars);
            return ([(header::CONTENT_TYPE, "application/javascript")], final_code)
                .into_response();
        }
    }

    // Phase 12: SPA Routing Fallback (History API fallback)
    // ponytail: if target doesn't exist, but it's an HTML/navigation request, serve index.html
    if !target.exists() && is_spa_fallback_request(req.method(), path, ext, req.headers()) {
        let index_html = state.root.join("index.html");
        if let Ok(content) = tokio::fs::read_to_string(&index_html).await {
            return Html(inject_hmr(&content)).into_response();
        }
    }

    next.run(req).await
}

fn is_spa_fallback_request(
    method: &axum::http::Method,
    path: &str,
    ext: &str,
    headers: &axum::http::HeaderMap,
) -> bool {
    // "/__*" and "/@*" are internal routes (HMR client, virtual modules), never app pages.
    if method != axum::http::Method::GET || path.starts_with("/__") || path.starts_with("/@") {
        return false;
    }
    let is_html_req = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("text/html"))
        .unwrap_or(false);
    let has_no_ext = ext.is_empty();
    is_html_req || has_no_ext
}

struct CliArgs {
    port: u16,
    root: PathBuf,
    open: bool,
    proxies: Vec<(String, String)>,
    https: bool,
    build: bool,
}

fn parse_args() -> CliArgs {
    // ponytail: std::env::args covers this, clap is 50 crates for a few flags
    let mut args_out = CliArgs {
        port: 3000,
        root: PathBuf::from("."),
        open: false,
        proxies: Vec::new(),
        https: false,
        build: false,
    };
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "build" | "--build" => {
                args_out.build = true;
            }
            "--port" | "-p" if i + 1 < args.len() => {
                args_out.port = args[i + 1].parse().unwrap_or(3000);
                i += 1;
            }
            "--root" | "-r" if i + 1 < args.len() => {
                args_out.root = PathBuf::from(&args[i + 1]);
                i += 1;
            }
            "--open" | "-o" => {
                args_out.open = true;
            }
            // Phase 13: --https for local SSL/TLS dev server
            "--https" => {
                args_out.https = true;
            }
            // Phase 10: --proxy /api=http://localhost:8080
            "--proxy" if i + 1 < args.len() => {
                if let Some((prefix, target)) = args[i + 1].split_once('=') {
                    args_out.proxies.push((prefix.to_string(), target.to_string()));
                }
                i += 1;
            }
            "--help" | "-h" => {
                println!("Usage: rush [build] [--port <port>] [--root <dir>] [--open] [--https] [--proxy <prefix>=<target>]");
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }
    args_out
}

fn should_ignore(path: &Path) -> bool {
    // ponytail: ignore common noise directories without a full .gitignore parser
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s == "node_modules" || s == ".git" || s == "target"
    })
}

/// OS watcher that drops access events. inotify reports every open/read, so without this
/// the server reading a file to serve it would broadcast a reload for that file.
struct NoAccessWatcher(notify::RecommendedWatcher);

impl notify::Watcher for NoAccessWatcher {
    fn new<F: notify::EventHandler>(mut handler: F, config: notify::Config) -> notify::Result<Self> {
        let filter = move |res: notify::Result<notify::Event>| {
            if !matches!(&res, Ok(e) if e.kind.is_access()) {
                handler.handle_event(res);
            }
        };
        notify::RecommendedWatcher::new(filter, config).map(Self)
    }

    fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
        self.0.watch(path, mode)
    }

    fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        self.0.unwatch(path)
    }

    fn kind() -> notify::WatcherKind {
        notify::RecommendedWatcher::kind()
    }
}

#[tokio::main]
async fn main() {
    let cli = parse_args();
    let root = match cli.root.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            log::error(&format!("cannot resolve root directory {}: {}", cli.root.display(), e));
            std::process::exit(1);
        }
    };

    // Phase 16: Production Build command
    if cli.build {
        if let Err(e) = build::run_build(&root) {
            log::error(&format!("build failed: {e}"));
            std::process::exit(1);
        }
        return;
    }

    // Phase 3: Pre-bundle node_modules dependencies via esbuild
    if let Err(e) = resolve::prebundle(&root) {
        log::warn(&format!("prebundle notice: {e}"));
    }

    let (tx, _rx) = broadcast::channel::<String>(16);
    let graph = Arc::new(RwLock::new(hmr::ModuleGraph::new()));

    // Spawn debounced file watcher in background thread
    let watcher_root = root.clone();
    let watcher_tx = tx.clone();
    let watcher_graph = graph.clone();
    std::thread::spawn(move || {
        let (sync_tx, sync_rx) = std::sync::mpsc::channel();
        let config = notify_debouncer_mini::Config::default().with_timeout(Duration::from_millis(50));
        let mut debouncer = match new_debouncer_opt::<_, NoAccessWatcher>(config, sync_tx) {
            Ok(d) => d,
            Err(e) => {
                log::error(&format!("watcher error: {}", e));
                return;
            }
        };

        if let Err(e) = debouncer.watcher().watch(&watcher_root, RecursiveMode::Recursive) {
            log::error(&format!("watcher register error: {}", e));
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
                        log::event("css update:", &rel);
                        let msg = serde_json::json!({
                            "type": "css",
                            "path": hmr::normalize_path(&rel),
                            "timestamp": now,
                        });
                        watcher_tx.send(msg.to_string()).ok();
                    } else if matches!(ext, "js" | "ts" | "jsx" | "tsx") {
                        let boundaries = watcher_graph.write().unwrap().invalidate(&rel, now);
                        if let Some(targets) = boundaries {
                            log::event("hot update:", &format!("{:?}", targets));
                            for target in targets {
                                let msg = serde_json::json!({
                                    "type": "js-update",
                                    "path": target,
                                    "timestamp": now,
                                });
                                watcher_tx.send(msg.to_string()).ok();
                            }
                        } else {
                            log::event("reload:", &rel);
                            let msg = serde_json::json!({
                                "type": "full-reload",
                                "path": hmr::normalize_path(&rel),
                            });
                            watcher_tx.send(msg.to_string()).ok();
                        }
                    } else {
                        log::event("reload:", &rel);
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

    // Phase 9: Load .env file
    let env_vars = Arc::new(env::load_env(&root));
    if env_vars.len() > 3 {
        // 3 builtins (DEV, PROD, MODE) are always present; log only if user vars found
        log::ok(&format!("loaded {} env vars from .env", env_vars.len() - 3));
    }

    // Phase 10: Proxy config
    let proxies = Arc::new(cli.proxies);
    for (prefix, target) in proxies.iter() {
        log::info(&format!("proxy: {} -> {}", prefix, target));
    }

    // Phase 11: Load path aliases from tsconfig.json
    let aliases = Arc::new(alias::load_aliases(&root));
    if !aliases.is_empty() {
        log::ok(&format!("loaded {} path aliases from tsconfig.json", aliases.len()));
    }

    let state = AppState {
        root: root.clone(),
        tx,
        graph,
        source_maps,
        env_vars,
        proxies,
        aliases,
    };

    let app = Router::new()
        .route("/__hmr", get(ws_handler))
        .route("/__hmr.js", get(hmr_client))
        .route("/@react-refresh", get(react_refresh_runtime))
        .fallback_service(ServeDir::new(&state.root))
        .layer(from_fn_with_state(state.clone(), request_middleware))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], cli.port));
    let proto = if cli.https { "https" } else { "http" };
    let url = format!("{proto}://localhost:{}/", cli.port);
    println!();
    println!("  {}{}🚀 Rush dev server running at:{}", log::CYAN, log::BOLD, log::RESET);
    println!("  {}>{} Local:   {}{}{}", log::GRAY, log::RESET, log::GREEN, url, log::RESET);
    println!("  {}>{} Network: {}{proto}://127.0.0.1:{}/{}", log::GRAY, log::RESET, log::GREEN, cli.port, log::RESET);
    println!("  {}>{} Root:    {}{}{}", log::GRAY, log::RESET, log::GRAY, root.display(), log::RESET);
    if cli.https {
        println!("  {}>{} TLS:     {}Self-signed certificate (accept browser warning){}", log::GRAY, log::RESET, log::YELLOW, log::RESET);
    }
    println!();

    // Phase 6: Auto-open browser
    // ponytail: platform-specific std::process::Command, no opener crate needed
    if cli.open {
        #[cfg(target_os = "windows")]
        { let _ = std::process::Command::new("cmd").args(["/C", "start", &url]).spawn(); }
        #[cfg(target_os = "macos")]
        { let _ = std::process::Command::new("open").arg(&url).spawn(); }
        #[cfg(target_os = "linux")]
        { let _ = std::process::Command::new("xdg-open").arg(&url).spawn(); }
    }

    if cli.https {
        // Phase 13: HTTPS dev server via self-signed cert
        // ponytail: generate self-signed cert in memory with rcgen, 0 config needed
        let certified_key = rcgen::generate_simple_self_signed(vec![
            "localhost".to_string(),
            "127.0.0.1".to_string(),
        ])
        .expect("failed to generate self-signed certificate");
        let cert_pem = certified_key.cert.pem();
        let key_pem = certified_key.signing_key.serialize_pem();

        let tls_config = RustlsConfig::from_pem(cert_pem.into_bytes(), key_pem.into_bytes())
            .await
            .expect("failed to load TLS certificate");

        axum_server::bind_rustls(addr, tls_config)
            .serve(app.into_make_service())
            .await
            .unwrap();
    } else {
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                log::error(&format!("failed to bind to {}: {}", addr, e));
                std::process::exit(1);
            }
        };
        axum::serve(listener, app).await.unwrap();
    }
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

    #[test]
    fn test_is_spa_fallback_request() {
        use axum::http::{HeaderMap, HeaderValue, Method};

        let mut headers = HeaderMap::new();
        // Navigation to /dashboard with no extension
        assert!(is_spa_fallback_request(&Method::GET, "/dashboard", "", &headers));

        // Navigation with text/html Accept header
        headers.insert(axum::http::header::ACCEPT, HeaderValue::from_static("text/html,application/xhtml+xml"));
        assert!(is_spa_fallback_request(&Method::GET, "/users/profile", "", &headers));
        assert!(is_spa_fallback_request(&Method::GET, "/some/page.html", "html", &headers));

        // Asset/module request with extension
        let mut asset_headers = HeaderMap::new();
        asset_headers.insert(axum::http::header::ACCEPT, HeaderValue::from_static("*/*"));
        assert!(!is_spa_fallback_request(&Method::GET, "/style.css", "css", &asset_headers));
        assert!(!is_spa_fallback_request(&Method::GET, "/app.js", "js", &asset_headers));

        // Non-GET requests should never trigger fallback
        assert!(!is_spa_fallback_request(&Method::POST, "/api/login", "", &headers));

        // Internal endpoints should never trigger fallback
        assert!(!is_spa_fallback_request(&Method::GET, "/__hmr", "", &headers));
        assert!(!is_spa_fallback_request(&Method::GET, "/@react-refresh", "", &headers));
    }

    #[test]
    fn test_rcgen_cert_generation() {
        let certified_key = rcgen::generate_simple_self_signed(vec![
            "localhost".to_string(),
            "127.0.0.1".to_string(),
        ])
        .expect("failed to generate certificate");
        let cert_pem = certified_key.cert.pem();
        let key_pem = certified_key.signing_key.serialize_pem();
        assert!(cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(cert_pem.contains("END CERTIFICATE"));
        assert!(key_pem.contains("PRIVATE KEY"));
    }
}
