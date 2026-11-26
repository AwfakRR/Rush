//! CSS pipeline integration against project-local Node dependencies.
#![cfg(unix)]

mod common;

use common::{rush, Fixture, Server};
use futures_util::StreamExt;
use serde_json::Value;
use std::{path::Path, process::Command, time::Duration};
use tokio_tungstenite::connect_async;

#[derive(Clone, Copy)]
enum Tailwind {
    V3,
    V4,
}

fn project(version: Tailwind) -> Option<Fixture> {
    let modules = Path::new(env!("CARGO_MANIFEST_DIR")).join("example/node_modules");
    let tailwind = match version {
        Tailwind::V3 => ["tailwindcss-v3"].as_slice(),
        Tailwind::V4 => ["tailwindcss", "@tailwindcss/postcss"].as_slice(),
    };
    if Command::new("node").arg("--version").output().is_err()
        || ![
            "esbuild",
            "postcss",
            "postcss-load-config",
            "postcss-modules",
        ]
        .iter()
        .chain(tailwind)
        .all(|name| modules.join(name).join("package.json").is_file())
    {
        eprintln!("skipping: Node.js and example dependencies needed (npm ci --prefix example)");
        return None;
    }
    let fixture = Fixture::new();
    std::os::unix::fs::symlink(modules, fixture.path().join("node_modules")).unwrap();
    fixture
        .write("package.json", r#"{"type":"module"}"#)
        .write(
            "index.html",
            r#"<html><head><link rel="stylesheet" href="/style.css"></head><body><script type="module" src="/main.js"></script></body></html>"#,
        )
        .write("side-effect.css", ".global { display: grid; }")
        .write("first.module.css", ".button { color: red; }")
        .write("second.module.css", ".button { color: blue; }")
        .write("main.js", ENTRY);
    match version {
        Tailwind::V3 => fixture
            .write(
                ".postcssrc.mjs",
                "export default { plugins: { 'tailwindcss-v3': { content: ['./main.js'] } } };",
            )
            .write("style.css", "@tailwind utilities;"),
        Tailwind::V4 => fixture
            .write(
                "postcss.config.cjs",
                "module.exports = { plugins: { '@tailwindcss/postcss': {} } };",
            )
            .write(
                "style.css",
                "@import \"tailwindcss\";\n@source \"./templates\";",
            )
            .write(".gitignore", "/templates\n/dist\n")
            .write("templates/email.html", "<p class=\"italic\">Message</p>"),
    };
    Some(fixture)
}

const ENTRY: &str = "import styles, { button } from './first.module.css';\n\
    import './side-effect.css';\n\
    document.body.classList.add(styles.button, button, 'font-bold');\n\
    if (import.meta.hot) import.meta.hot.accept();";

async fn classes(server: &Server, path: &str) -> Value {
    let json = server.get_text(&format!("{path}?rush-css-classes")).await;
    serde_json::from_str(&json).unwrap()
}

#[tokio::test]
async fn dev_modules_are_scoped_and_tailwind_cache_tracks_content() {
    check_dev(Tailwind::V3).await;
}

#[tokio::test]
async fn tailwind_v4_dev_scans_content_and_hot_swaps_css() {
    check_dev(Tailwind::V4).await;
}

async fn check_dev(version: Tailwind) {
    let Some(fixture) = project(version) else {
        return;
    };
    let server = Server::start(fixture.path(), &[]);
    assert!(server.get_text("/style.css").await.contains(".font-bold"));
    if matches!(version, Tailwind::V4) {
        assert!(server.get_text("/style.css").await.contains(".italic"));
    }
    assert!(server
        .get_text("/side-effect.css?import")
        .await
        .contains("export {};"));
    server.get_text("/main.js").await;

    let script = reqwest::Client::new()
        .get(server.url("/first.module.css"))
        .header("Sec-Fetch-Dest", "script")
        .send()
        .await
        .unwrap();
    assert_eq!(script.headers()["content-type"], "application/javascript");
    let script = script.text().await.unwrap();
    assert!(script.contains("export default classes"));
    assert!(script.contains("as button"));
    let first = classes(&server, "/first.module.css").await;
    let second = classes(&server, "/second.module.css").await;
    let scoped = first["button"].as_str().unwrap();
    assert_ne!(scoped, "button");
    assert_ne!(first["button"], second["button"]);
    let style = server.get("/first.module.css").await;
    assert_eq!(style.headers()["content-type"], "text/css");
    assert!(style.text().await.unwrap().contains(&format!(".{scoped}")));

    let (mut ws, _) = connect_async(server.ws_url()).await.unwrap();
    fixture.write("first.module.css", ".button { color: green; }");
    let message = next_message(&mut ws).await;
    assert_eq!(message["type"], "css");
    assert_eq!(message["path"], "/first.module.css");
    assert_eq!(first, classes(&server, "/first.module.css").await);
    assert!(server
        .get_text("/first.module.css")
        .await
        .contains("color: green"));

    fixture.write("first.module.css", ".updated { color: green; }");
    assert_eq!(next_message(&mut ws).await["type"], "css");
    let updated = classes(&server, "/first.module.css").await;
    assert!(updated["button"].is_null());
    assert!(updated["updated"].is_string());

    fixture.write("main.js", ENTRY.replace("font-bold", "font-black"));
    assert_eq!(next_message(&mut ws).await["type"], "css");
    assert_eq!(next_message(&mut ws).await["type"], "js-update");
    let css = server.get_text("/style.css").await;
    assert!(css.contains(".font-black"));
    assert!(!css.contains(".font-bold"));
    if matches!(version, Tailwind::V4) {
        fixture.write("templates/email.html", "<p class=\"underline\">Message</p>");
        assert_eq!(next_message(&mut ws).await["type"], "css");
        let css = server.get_text("/style.css").await;
        assert!(css.contains(".underline"));
        assert!(!css.contains(".italic"));
    }
}

async fn next_message(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    let message = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("HMR message timed out")
        .unwrap()
        .unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

#[tokio::test]
async fn build_links_bundled_css_with_the_same_scoped_names_as_dev() {
    check_build(Tailwind::V3).await;
}

#[tokio::test]
async fn tailwind_v4_build_emits_utilities_and_scoped_module_css() {
    check_build(Tailwind::V4).await;
}

async fn check_build(version: Tailwind) {
    let Some(fixture) = project(version) else {
        return;
    };
    let scoped = {
        let server = Server::start(fixture.path(), &[]);
        classes(&server, "/first.module.css").await["button"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let output = rush()
        .args(["build", "--root"])
        .arg(fixture.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dist = fixture.path().join("dist");
    let html = std::fs::read_to_string(dist.join("index.html")).unwrap();
    let bundled_path = html
        .split('"')
        .find(|path| path.starts_with("./assets/") && path.ends_with(".css"))
        .expect("built stylesheet link");
    let css = std::fs::read_to_string(dist.join(bundled_path)).unwrap();
    assert!(css.contains(&format!(".{scoped}")), "{css}");
    assert!(css.contains(".global{display:grid}"), "{css}");
    let js_path = html
        .split('"')
        .find(|path| path.starts_with("./assets/") && path.ends_with(".js"))
        .unwrap();
    assert!(std::fs::read_to_string(dist.join(js_path))
        .unwrap()
        .contains(&scoped));
    let linked = std::fs::read_to_string(dist.join("style.css")).unwrap();
    assert!(linked.contains(".font-bold"), "{linked}");
    assert!(!linked.contains("@tailwind"), "{linked}");
    if matches!(version, Tailwind::V4) {
        assert!(linked.contains(".italic"), "{linked}");
        assert!(!linked.contains("@import"), "{linked}");
        assert!(!linked.contains("@source"), "{linked}");
    }
}
