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
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::broadcast;
use tower_http::services::ServeDir;

mod transform;

#[derive(Clone)]
struct AppState {
    root: PathBuf,
    tx: broadcast::Sender<String>,
}

const HMR_CLIENT: &str = r#"(() => {
  const socket = new WebSocket(`ws://${location.host}/__hmr`);
  socket.addEventListener('open', () => console.log('[rush] connected'));
  socket.addEventListener('message', ({ data }) => {
    console.log('[rush] update:', data);
    if (data.endsWith('.css') || data.startsWith('css:')) {
      const links = document.querySelectorAll('link[rel="stylesheet"]');
      for (const link of links) {
        const url = new URL(link.href);
        url.searchParams.set('t', Date.now());
        link.href = url.href;
      }
    } else {
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

    // HTML: inject HMR client script
    if target.is_file() && ext == "html" {
        if let Ok(content) = tokio::fs::read_to_string(&target).await {
            return Html(inject_hmr(&content)).into_response();
        }
    }

    // TS/TSX/JSX: transpile on the fly via oxc
    // ponytail: no caching of transpile results — files are small, oxc is microseconds.
    // Add in-memory cache if profiler says transpilation is the bottleneck.
    if target.is_file() && matches!(ext, "ts" | "tsx" | "jsx") {
        if let Ok(source) = tokio::fs::read_to_string(&target).await {
            let fname = target.file_name().unwrap().to_string_lossy().to_string();
            match transform::transpile(&fname, &source) {
                Ok((code, _map)) => {
                    // ponytail: source maps deferred to Phase 5, serve JS only for now
                    return ([(header::CONTENT_TYPE, "application/javascript")], code)
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

    let (tx, _rx) = broadcast::channel::<String>(16);

    // Spawn debounced file watcher in background thread
    let watcher_root = root.clone();
    let watcher_tx = tx.clone();
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

                    println!("[rush] changed: {}", rel);
                    watcher_tx.send(rel).ok();
                }
            }
        }
    });

    let state = AppState {
        root: root.clone(),
        tx,
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
