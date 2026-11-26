//! Test project compilers using the dependencies installed by npm ci --prefix example.
#![cfg(unix)]

mod common;

use common::{Fixture, Server, rush};
use futures_util::StreamExt;
use serde_json::Value;
use std::{fs, path::Path, process::Command, time::Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};

async fn check_framework(
    framework: &str,
    component: &str,
    compiler: &str,
    runtime_url: &str,
    hmr_code: &str,
    color: &str,
) {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let modules = repo.join("example/node_modules");
    if !modules.join(compiler).is_dir()
        || !Command::new("node")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    {
        eprintln!("skipping: Node.js and npm ci --prefix example are required");
        return;
    }
    let fixture = Fixture::new();
    let project_modules = fixture.path().join("node_modules");
    fs::create_dir(&project_modules).unwrap();
    for package in fs::read_dir(&modules).unwrap() {
        let package = package.unwrap();
        if !package.file_name().to_string_lossy().starts_with('.') {
            std::os::unix::fs::symlink(package.path(), project_modules.join(package.file_name()))
                .unwrap();
        }
    }
    for entry in fs::read_dir(repo.join(format!("example-{framework}"))).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            fixture.write(
                entry.file_name().to_str().unwrap(),
                fs::read(entry.path()).unwrap(),
            );
        }
    }
    let source = fs::read_to_string(fixture.path().join(component)).unwrap();
    {
        let server = Server::start(fixture.path(), &[]);
        let response = server.get(&format!("/{component}")).await;
        let status = response.status();
        let content_type = response.headers()["content-type"]
            .to_str()
            .unwrap()
            .to_string();
        let js = response.text().await.unwrap();
        assert!(status.is_success(), "{status}: {js}");
        assert!(content_type.contains("application/javascript"));
        assert!(js.contains(hmr_code), "{js}");
        assert!(!js.contains(": number") && !js.contains("<number>"), "{js}");
        assert!(js.contains("/node_modules/.rush/deps/"), "{js}");
        assert!(!server.get_text(runtime_url).await.is_empty());

        let (mut ws, _) = connect_async(server.ws_url()).await.unwrap();
        let mut edited = source.replace("Count with", "Updated count with");
        let path = format!("/{component}");
        for style_only in [false, true] {
            if style_only {
                if framework == "solid" {
                    break;
                }
                edited = edited.replace(color, "#9d174d");
            }
            fixture.write(component, &edited);
            let message = tokio::time::timeout(Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let Message::Text(message) = message else {
                panic!("expected update: {message:?}");
            };
            let update: Value = serde_json::from_str(message.as_str()).unwrap();
            assert_eq!(update["type"], "js-update", "{update}");
            assert_eq!(update["path"], path, "{update}");
            let updated = server
                .get_text(&format!("{path}?t={}", update["timestamp"]))
                .await;
            assert!(updated.contains("Updated count with"), "{updated}");
            if style_only {
                assert!(updated.contains("#9d174d"), "{updated}");
            }
        }
    }
    fixture.write(component, source);
    let result = rush()
        .args(["build", "--root"])
        .arg(fixture.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let dist = fixture.path().join("dist");
    let html = fs::read_to_string(dist.join("index.html")).unwrap();
    assert!(html.contains("src=\"./assets/"), "{html}");
    assert!(html.contains("href=\"./assets/"), "{html}");
    let assets = fs::read_dir(dist.join("assets"))
        .unwrap()
        .map(|entry| entry.unwrap().path());
    let mut found_js = false;
    let mut found_css = false;
    for file in assets {
        match file.extension().and_then(|ext| ext.to_str()) {
            Some("js") => {
                let js = fs::read_to_string(file).unwrap();
                found_js |= js.contains("Count with");
                assert!(
                    !js.contains(hmr_code),
                    "{framework} HMR leaked into production"
                );
            }
            Some("css") => {
                let css = fs::read_to_string(file).unwrap();
                found_css |= css.contains(color);
                if framework == "vue" {
                    assert!(css.contains("[data-v-"));
                }
            }
            _ => {}
        }
    }
    assert!(
        found_js && found_css,
        "{framework}: missing compiled JS or CSS"
    );
}

#[tokio::test]
async fn vue_serves_updates_and_builds() {
    check_framework(
        "vue",
        "Counter.vue",
        "@vue/compiler-sfc",
        "/node_modules/.rush/deps/vue.js",
        "__rush_vue_hmr",
        "#047857",
    )
    .await;
}

#[tokio::test]
async fn svelte_serves_updates_and_builds() {
    check_framework(
        "svelte",
        "Counter.svelte",
        "svelte",
        "/node_modules/.rush/deps/svelte__internal__client.js",
        "import.meta.hot.accept",
        "#c2410c",
    )
    .await;
}

#[tokio::test]
async fn solid_serves_updates_and_builds() {
    check_framework(
        "solid",
        "Counter.tsx",
        "babel-preset-solid",
        "/node_modules/.rush/deps/solid-js__web.js",
        "import.meta.hot.accept",
        "#1d4ed8",
    )
    .await;
}

#[tokio::test]
async fn missing_compiler_reports_the_install_command() {
    if !Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return;
    }
    let fixture = Fixture::new();
    fixture.write("Counter.vue", "<template><p>Counter</p></template>");
    let server = Server::start(fixture.path(), &[]);
    let response = server.get("/Counter.vue").await;
    assert_eq!(response.status(), 500);
    assert!(
        response
            .text()
            .await
            .unwrap()
            .contains("npm install -D @vue/compiler-sfc")
    );
}
