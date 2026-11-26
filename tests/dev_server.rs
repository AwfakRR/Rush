//! End-to-end checks against a running dev server: transpilation, resolution, serving rules, proxy.
mod common;

use common::{Fixture, Server};
use reqwest::header::{ACCEPT, CONTENT_TYPE};

const JS: &str = "application/javascript";

fn content_type(resp: &reqwest::Response) -> String {
    resp.headers()
        .get(CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default()
}

fn project() -> Fixture {
    let f = Fixture::new();
    f.write("index.html", "<html><head><title>t</title></head><body><script type=\"module\" src=\"/main.js\"></script></body></html>")
        .write("sub/index.html", "<html><body>sub</body></html>")
        .write("tsconfig.json", r#"{"compilerOptions":{"paths":{"@/*":["./src/*"]}}}"#)
        .write(".env", "RUSH_APP_NAME=Rush Test\nSECRET_TOKEN=nope\n")
        .write(
            "main.js",
            "import React from 'react';\nimport { greet } from '@/greet.ts';\nimport './local.js';\nconsole.log(greet(), import.meta.env.RUSH_APP_NAME, import.meta.env.DEV, import.meta.env.SECRET_TOKEN);\n",
        )
        .write(
            "app.tsx",
            "import { useState } from 'react';\nimport { Button } from '@tanstack/ui';\nimport { greet } from '@/greet.ts';\ninterface Props { name: string }\nexport function App(props: Props) { const [n] = useState<number>(0); return <Button>{greet()} {props.name} {n} {import.meta.env.MODE}</Button>; }\n",
        )
        .write("src/greet.ts", "export const greet = (): string => 'hi';\n")
        .write("broken.ts", "const = ;\n")
        .write("data.json", "{\n  \"answer\": 42\n}\n")
        .write("logo.png", [0x89u8, b'P', b'N', b'G'])
        .write("style.css", "body { color: red; }\n");
    f
}

#[tokio::test]
async fn serves_html_with_hmr_client_injected() {
    let f = project();
    let s = Server::start(f.path(), &[]);

    let html = s.get_text("/").await;
    let tag = r#"<script type="module" src="/__hmr.js"></script>"#;
    assert!(
        html.find(tag).unwrap() < html.find("</head>").unwrap(),
        "{html}"
    );

    // Directory requests serve the nested index.html, also injected.
    assert!(s.get_text("/sub/").await.contains(tag));

    let client = s.get("/__hmr.js").await;
    assert_eq!(content_type(&client), JS);
    assert!(client.text().await.unwrap().contains("/__hmr`"));

    let refresh = s.get_text("/@react-refresh").await;
    assert!(refresh.contains("performReactRefresh"));
    assert!(refresh.contains("createSignatureFunctionForTransform"));
    assert!(refresh.contains("scheduleRefresh"));
    assert!(
        html.find("refresh.injectIntoGlobalHook(window)").unwrap()
            < html.find(r#"src="/main.js""#).unwrap(),
        "{html}"
    );
}

#[tokio::test]
async fn transpiles_tsx_and_resolves_imports() {
    let f = project();
    let s = Server::start(f.path(), &[]);

    let resp = s.get("/app.tsx").await;
    assert_eq!(content_type(&resp), JS);
    let code = resp.text().await.unwrap();

    // TypeScript and JSX are gone.
    assert!(!code.contains("interface Props"), "{code}");
    assert!(!code.contains("<Button>"), "{code}");
    assert!(!code.contains("<number>"), "{code}");
    // Bare imports point at pre-bundled deps, including scoped packages and the JSX runtime.
    assert!(
        code.contains(r#""/node_modules/.rush/deps/react.js""#),
        "{code}"
    );
    assert!(
        code.contains(r#""/node_modules/.rush/deps/@tanstack__ui.js""#),
        "{code}"
    );
    assert!(
        code.contains(r#""/node_modules/.rush/deps/react__jsx-runtime.js""#),
        "{code}"
    );
    // tsconfig path alias is rewritten.
    assert!(code.contains(r#""./src/greet.ts""#), "{code}");
    assert!(!code.contains("@/greet"), "{code}");
    // HMR context, React Fast Refresh registration, env define, source map link.
    assert!(code.starts_with("// [rush] HMR context"), "{code}");
    assert!(code.contains("window.__rush_hot.get('/app.tsx')"), "{code}");
    assert!(code.contains("$RefreshReg$"), "{code}");
    assert!(
        code.contains(r#"import __rush_refresh from "/@react-refresh""#),
        "{code}"
    );
    assert!(
        code.contains("import.meta.hot._refreshUrl = import.meta.url"),
        "{code}"
    );
    assert!(code.contains(r#""development""#), "{code}");
    assert!(code.contains("//# sourceMappingURL=/app.tsx.map"), "{code}");

    let map = s.get("/app.tsx.map").await;
    assert_eq!(content_type(&map), "application/json");
    let map: serde_json::Value = serde_json::from_str(&map.text().await.unwrap()).unwrap();
    assert_eq!(map["version"], 3);
}

#[tokio::test]
async fn rewrites_plain_js_and_replaces_env() {
    let f = project();
    let s = Server::start(f.path(), &[]);

    let resp = s.get("/main.js").await;
    assert_eq!(content_type(&resp), JS);
    let code = resp.text().await.unwrap();
    assert!(
        code.contains(r#"from "/node_modules/.rush/deps/react.js""#),
        "{code}"
    );
    assert!(code.contains(r#"from "./src/greet.ts""#), "{code}");
    assert!(code.contains("import './local.js';"), "{code}");
    assert!(code.contains(r#""Rush Test""#), "{code}");
    assert!(
        code.contains("console.log(greet(), \"Rush Test\", true,"),
        "{code}"
    );
    // Non-RUSH_ vars are never exposed.
    assert!(code.contains("import.meta.env.SECRET_TOKEN"), "{code}");
    assert!(!code.contains("nope"), "{code}");
}

#[tokio::test]
async fn transpile_error_returns_console_error_module() {
    let f = project();
    let s = Server::start(f.path(), &[]);

    let resp = s.get("/broken.ts").await;
    assert!(resp.status().is_success());
    assert_eq!(content_type(&resp), JS);
    let body = resp.text().await.unwrap();
    assert!(
        body.starts_with("console.error(`[rush] Parse error in broken.ts"),
        "{body}"
    );
}

#[tokio::test]
async fn serves_json_and_assets_as_modules() {
    let f = project();
    let s = Server::start(f.path(), &[]);

    let json = s.get("/data.json").await;
    assert_eq!(content_type(&json), JS);
    assert_eq!(
        json.text().await.unwrap(),
        "export default {\n  \"answer\": 42\n};\n"
    );

    let as_module = s.get("/logo.png?import").await;
    assert_eq!(content_type(&as_module), JS);
    assert_eq!(
        as_module.text().await.unwrap(),
        "export default \"/logo.png\";\n"
    );

    let script_dest = reqwest::Client::new()
        .get(s.url("/logo.png"))
        .header("sec-fetch-dest", "script")
        .send()
        .await
        .unwrap();
    assert_eq!(
        script_dest.text().await.unwrap(),
        "export default \"/logo.png\";\n"
    );

    let raw = s.get("/logo.png").await;
    assert_eq!(content_type(&raw), "image/png");
    assert_eq!(
        raw.bytes().await.unwrap().as_ref(),
        &[0x89, b'P', b'N', b'G']
    );

    let css = s.get("/style.css").await;
    assert_eq!(content_type(&css), "text/css");
    assert_eq!(css.text().await.unwrap(), "body { color: red; }\n");
}

#[tokio::test]
async fn spa_fallback_only_for_navigations() {
    let f = project();
    let s = Server::start(f.path(), &[]);
    let client = reqwest::Client::new();

    let nav = s.get_text("/dashboard/settings").await;
    assert!(
        nav.contains("<title>t</title>") && nav.contains("/__hmr.js"),
        "{nav}"
    );

    let html_accept = client
        .get(s.url("/old/page.html"))
        .header(ACCEPT, "text/html")
        .send()
        .await
        .unwrap();
    assert!(html_accept
        .text()
        .await
        .unwrap()
        .contains("<title>t</title>"));

    let missing_asset = client
        .get(s.url("/missing.js"))
        .header(ACCEPT, "*/*")
        .send()
        .await
        .unwrap();
    assert_eq!(missing_asset.status(), 404);

    let post = client.post(s.url("/dashboard")).send().await.unwrap();
    assert_ne!(post.status(), 200);
}

#[tokio::test]
async fn proxies_matching_prefix_to_upstream() {
    use axum::{extract::Request, Router};

    let upstream = tokio::net::TcpListener::bind(("127.0.0.1", common::free_port()))
        .await
        .unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();
    let app = Router::new().fallback(|req: Request| async move {
        (
            axum::http::StatusCode::IM_A_TEAPOT,
            format!("{} {}", req.method(), req.uri()),
        )
    });
    tokio::spawn(async move { axum::serve(upstream, app).await.unwrap() });

    let f = project();
    let dead = common::free_port();
    let s = Server::start(
        f.path(),
        &[
            "--proxy",
            &format!("/api=http://127.0.0.1:{upstream_port}/"),
            "--proxy",
            &format!("/down=http://127.0.0.1:{dead}"),
        ],
    );

    let resp = s.get("/api/users?id=7").await;
    assert_eq!(resp.status(), 418);
    assert_eq!(resp.text().await.unwrap(), "GET /users?id=7");

    let del = reqwest::Client::new()
        .delete(s.url("/api/x"))
        .send()
        .await
        .unwrap();
    assert_eq!(del.text().await.unwrap(), "DELETE /x");

    let down = s.get("/down/anything").await;
    assert_eq!(down.status(), 502);

    // Non-matching paths are still served locally.
    assert!(s.get_text("/main.js").await.contains("greet"));
}
