//! End-to-end HMR: edit files on disk and assert what the dev server broadcasts over /__hmr.
mod common;

use common::{Fixture, Server};
use futures_util::StreamExt;
use serde_json::Value;
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

fn project() -> Fixture {
    let f = Fixture::new();
    f.write("index.html", "<html><head></head><body><script type=\"module\" src=\"/main.js\"></script></body></html>")
        .write("tsconfig.json", r#"{"compilerOptions":{"paths":{"@/*":["./*"]}}}"#)
        .write("main.js", "import { createCounter } from './counter.ts';\nimport { App } from './App.tsx';\ncreateCounter(); App();\n")
        .write(
            "counter.ts",
            "import { step } from './util.ts';\nexport function createCounter(): number { return step; }\nif (import.meta.hot) { import.meta.hot.accept(); }\n",
        )
        .write("util.ts", "export const step: number = 1;\n")
        .write(
            "App.tsx",
            "import { label } from '@/label.ts';\nimport { step } from './nested/../util.ts';\nexport function App() { return <p>{label}{step}</p>; }\nimport.meta.hot.accept();\n",
        )
        .write("label.ts", "export const label = 'x';\n")
        .write("orphan.ts", "export const o = 1;\n")
        .write("style.css", "body {}\n");
    f
}

/// Start the server, request every module once (that is what populates the module graph), connect HMR.
async fn start(f: &Fixture) -> (Server, Ws) {
    let s = Server::start(f.path(), &[]);
    for m in [
        "/main.js",
        "/counter.ts",
        "/util.ts",
        "/App.tsx",
        "/label.ts",
    ] {
        s.get_text(m).await;
    }
    let (ws, _) = connect_async(s.ws_url()).await.expect("ws connect");
    // Let the watcher settle before the first edit.
    tokio::time::sleep(Duration::from_millis(200)).await;
    (s, ws)
}

/// Collect every message broadcast within a quiet window after an edit.
async fn edit_and_collect(f: &Fixture, ws: &mut Ws, rel: &str, content: &str) -> Vec<Value> {
    f.write(rel, content);
    let mut msgs = Vec::new();
    let first = tokio::time::timeout(Duration::from_secs(5), ws.next()).await;
    let mut next = first.ok().flatten();
    while let Some(Ok(Message::Text(t))) = next {
        msgs.push(serde_json::from_str(t.as_str()).unwrap());
        next = tokio::time::timeout(Duration::from_millis(400), ws.next())
            .await
            .ok()
            .flatten();
    }
    assert!(!msgs.is_empty(), "no HMR message after editing {rel}");
    msgs
}

fn kinds(msgs: &[Value]) -> Vec<(String, String)> {
    let mut v: Vec<_> = msgs
        .iter()
        .map(|m| {
            (
                m["type"].as_str().unwrap().to_string(),
                m["path"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

fn pair(t: &str, p: &str) -> (String, String) {
    (t.to_string(), p.to_string())
}

#[tokio::test]
async fn self_accepting_module_hot_updates_itself() {
    let f = project();
    let (_s, mut ws) = start(&f).await;
    let msgs = edit_and_collect(&f, &mut ws, "counter.ts", "import { step } from './util.ts';\nexport function createCounter(): number { return step + 1; }\nimport.meta.hot.accept();\n").await;
    assert_eq!(kinds(&msgs), vec![pair("js-update", "/counter.ts")]);
    assert!(msgs[0]["timestamp"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn react_component_hot_updates_without_manual_acceptance() {
    let initial =
        "import { label } from './label.ts';\nexport function App() { return <p>{label}</p>; }\n";
    let f = project();
    f.write("App.tsx", initial);
    let (s, mut ws) = start(&f).await;
    assert!(s
        .get_text("/App.tsx")
        .await
        .contains("import.meta.hot._refreshUrl = import.meta.url"));
    assert!(!s.get_text("/label.ts").await.contains("/@react-refresh"));

    let updated = initial
        .replace("<p>", "<strong>")
        .replace("</p>", "</strong>");
    let msgs = edit_and_collect(&f, &mut ws, "App.tsx", &updated).await;
    assert_eq!(kinds(&msgs), vec![pair("js-update", "/App.tsx")]);

    let msgs = edit_and_collect(&f, &mut ws, "label.ts", "export const label = 'new';").await;
    assert_eq!(kinds(&msgs), vec![pair("js-update", "/App.tsx")]);
    let timestamp = msgs.last().unwrap()["timestamp"].as_u64().unwrap();
    let updated = s.get_text(&format!("/App.tsx?t={timestamp}")).await;
    assert!(
        updated.contains(&format!("from \"./label.ts?t={timestamp}\"")),
        "{updated}"
    );
}

#[tokio::test]
async fn dependency_change_propagates_to_accepting_importers() {
    // util.ts is imported by counter.ts ('./util.ts') and App.tsx ('./nested/../util.ts'), both accepting.
    let f = project();
    let (_s, mut ws) = start(&f).await;
    let msgs = edit_and_collect(&f, &mut ws, "util.ts", "export const step: number = 2;\n").await;
    assert_eq!(
        kinds(&msgs),
        vec![
            pair("js-update", "/App.tsx"),
            pair("js-update", "/counter.ts")
        ]
    );
}

#[tokio::test]
async fn aliased_dependency_propagates_to_importer() {
    let f = project();
    let (_s, mut ws) = start(&f).await;
    let msgs = edit_and_collect(&f, &mut ws, "label.ts", "export const label = 'y';\n").await;
    assert_eq!(kinds(&msgs), vec![pair("js-update", "/App.tsx")]);
}

#[tokio::test]
async fn unaccepted_entry_triggers_full_reload() {
    let f = project();
    let (_s, mut ws) = start(&f).await;
    let msgs = edit_and_collect(&f, &mut ws, "main.js", "console.log('changed');\n").await;
    assert_eq!(kinds(&msgs), vec![pair("full-reload", "/main.js")]);
}

#[tokio::test]
async fn never_served_module_triggers_full_reload() {
    let f = project();
    let (_s, mut ws) = start(&f).await;
    let msgs = edit_and_collect(&f, &mut ws, "orphan.ts", "export const o = 2;\n").await;
    assert_eq!(kinds(&msgs), vec![pair("full-reload", "/orphan.ts")]);
}

#[tokio::test]
async fn css_change_sends_css_update() {
    let f = project();
    let (_s, mut ws) = start(&f).await;
    let msgs = edit_and_collect(&f, &mut ws, "style.css", "body { color: blue; }\n").await;
    assert_eq!(kinds(&msgs), vec![pair("css", "/style.css")]);
}

#[tokio::test]
async fn other_files_trigger_full_reload_and_node_modules_is_ignored() {
    let f = project();
    let (_s, mut ws) = start(&f).await;

    f.write("node_modules/pkg/index.js", "export default 1;\n");
    let ignored = tokio::time::timeout(Duration::from_millis(600), ws.next()).await;
    assert!(
        ignored.is_err(),
        "node_modules edit was broadcast: {ignored:?}"
    );

    let msgs = edit_and_collect(
        &f,
        &mut ws,
        "index.html",
        "<html><head></head><body>v2</body></html>",
    )
    .await;
    assert_eq!(kinds(&msgs), vec![pair("full-reload", "/index.html")]);
}

#[tokio::test]
async fn transpile_error_is_broadcast_and_replayed_for_overlay() {
    let f = project();
    f.write("bad.tsx", "export const = ;\n");
    let (s, mut ws) = start(&f).await;
    assert_eq!(
        s.get("/bad.tsx").await.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );
    let msg = next_message(&mut ws).await;
    assert_eq!(msg["type"], "error");
    assert_eq!(msg["file"], "bad.tsx");
    assert!(msg["message"]
        .as_str()
        .unwrap()
        .contains("Parse error in bad.tsx"));

    let (mut late, _) = connect_async(s.ws_url()).await.unwrap();
    assert_eq!(next_message(&mut late).await, msg);

    let recovered = edit_and_collect(&f, &mut ws, "bad.tsx", "export const good = 1;").await;
    assert_eq!(kinds(&recovered), vec![pair("full-reload", "/bad.tsx")]);
    let (mut fresh, _) = connect_async(s.ws_url()).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(300), fresh.next())
            .await
            .is_err()
    );
}

async fn next_message(ws: &mut Ws) -> Value {
    let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(msg.to_text().unwrap()).unwrap()
}

#[tokio::test]
async fn broken_save_reports_only_an_error_then_recovers() {
    let f = project();
    f.write(
        "App.tsx",
        "export default function App() { return <p>Before</p>; }",
    );
    let (_s, mut ws) = start(&f).await;
    let broken = "export default function App() { const broken = ; return <p>After</p>; }";

    for path in ["App.tsx", "unserved.tsx"] {
        let msgs = edit_and_collect(&f, &mut ws, path, broken).await;
        assert_eq!(kinds(&msgs), vec![pair("error", &format!("/{path}"))]);
        assert!(msgs[0]["message"]
            .as_str()
            .unwrap()
            .contains("Unexpected token"));
    }
    let fixed = "export default function App() { return <p>After</p>; }";
    let msgs = edit_and_collect(&f, &mut ws, "App.tsx", fixed).await;
    assert_eq!(kinds(&msgs), vec![pair("js-update", "/App.tsx")]);
}

#[tokio::test]
async fn serving_files_does_not_broadcast() {
    // Reading a file (open/close events) must not look like an edit, or pages reload themselves on load.
    let f = project();
    let (s, mut ws) = start(&f).await;
    for m in ["/", "/main.js", "/counter.ts", "/style.css"] {
        s.get_text(m).await;
    }
    let got = tokio::time::timeout(Duration::from_millis(600), ws.next()).await;
    assert!(got.is_err(), "serving files broadcast: {got:?}");
}
