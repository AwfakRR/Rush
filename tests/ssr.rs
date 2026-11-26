//! Exercise both SSR entry points with the real Rush binary and a Node consumer.
#![cfg(unix)]

mod common;

use common::{rush, Fixture, Server};
use futures_util::StreamExt;
use std::{fs, os::unix::fs::symlink, path::Path, process::Command, time::Duration};
use tokio_tungstenite::connect_async;

fn project() -> Option<Fixture> {
    let esbuild = Path::new(env!("CARGO_MANIFEST_DIR")).join("example/node_modules/esbuild");
    if !esbuild.is_dir() || Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipping: Node.js and npm ci --prefix example are required");
        return None;
    }
    let f = Fixture::new();
    f.write(
        "index.html",
        r#"<head></head><script type="module" src="/client.ts"></script>"#,
    )
    .write("client.ts", "import './style.css'; import './leaf.ts';")
    .write("style.css", "body { color: red; }")
    .write("leaf.ts", "export const label: string = 'First';")
    .write("entry.tsx", r#"
        /** @jsxRuntime classic */
        /** @jsx h */
        import { label } from './leaf';
        import mode from 'virtual:mode';
        import nodeOnly from 'ssr-external';
        function h(tag: string, _props: unknown, ...children: string[]) {
            return `<${tag}>${children.join('')}</${tag}>`;
        }
        export const render = () => <h1>{label}{' ('}{mode}{', '}{nodeOnly}{')'}</h1>;
    "#)
    .write("node_modules/ssr-external/index.js", "module.exports = require('node:path').basename('/node');")
    .write("rush.config.mjs", r#"
        export default ({ command, isSsrBuild }) => ({
          plugins: [{
            name: 'ssr-mode',
            resolveId(id, importer, { ssr }) {
              if (ssr && id === 'virtual:mode') return '\0mode';
            },
            load(id, { ssr }) {
              if (ssr && id === '\0mode') return 'export default "pending"';
            },
            transform(code, id, { ssr }) {
              if (ssr && id === '\0mode') return code.replace('pending', isSsrBuild ? 'build' : command);
            }
          }]
        });
    "#);
    symlink(esbuild, f.path().join("node_modules/esbuild")).unwrap();
    Some(f)
}

#[tokio::test]
async fn ssr_load_module_renders_and_invalidates_after_dependency_edit() {
    let Some(f) = project() else { return };
    f.write(
        "load.mjs",
        r#"
        import assert from 'node:assert/strict';
        import { writeFile } from 'node:fs/promises';
        import { setTimeout } from 'node:timers/promises';
        import { pathToFileURL } from 'node:url';
        const [helper, rush, origin] = process.argv.slice(2);
        const { createSsrLoader } = await import(pathToFileURL(helper));
        const loader = createSsrLoader({ origin, rush });
        try {
          const first = await loader.ssrLoadModule('entry.tsx');
          assert.equal(first.render(), '<h1>First (serve, node)</h1>');
          assert.equal(await loader.ssrLoadModule('entry.tsx'), first);
          const generation = async () => (await fetch(origin + '/__rush/ssr-version')).text();
          const before = await generation();
          await writeFile('leaf.ts', "export const label: string = 'Second';");
          const deadline = Date.now() + 5000;
          while (await generation() === before) {
            assert.ok(Date.now() < deadline, 'no SSR invalidation after dependency edit');
            await setTimeout(20);
          }
          const second = await loader.ssrLoadModule('entry.tsx');
          assert.notEqual(second, first);
          assert.equal(second.render(), '<h1>Second (serve, node)</h1>');
        } finally {
          await loader.close();
        }
        "#,
    );
    let server = Server::start(f.path(), &[]);
    server.get_text("/client.ts").await;
    let (mut ws, _) = connect_async(server.ws_url()).await.unwrap();
    let result = Command::new("node")
        .current_dir(f.path())
        .arg("load.mjs")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("ssr.mjs"))
        .arg(env!("CARGO_BIN_EXE_rush"))
        .arg(server.url(""))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let update = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(message)) = ws.next().await {
            if let Ok(update) = serde_json::from_slice::<serde_json::Value>(&message.into_data()) {
                if update["path"] == "/leaf.ts" {
                    return update;
                }
            }
        }
        panic!("HMR socket closed before the edit was reported");
    })
    .await
    .expect("client HMR after the SSR dependency edit");
    assert_eq!(update["type"], "full-reload");
}

#[test]
fn ssr_build_renders_and_manifest_points_to_client_assets() {
    let Some(f) = project() else { return };
    let result = rush()
        .args(["build", "--ssr", "entry.tsx", "--root"])
        .arg(f.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );

    let dist = f.path().join("dist");
    let output = Command::new("node")
        .current_dir(&dist)
        .args([
            "--input-type=module",
            "-e",
            "import { render } from './server/entry.mjs'; console.log(render());",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "<h1>First (build, node)</h1>"
    );

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(dist.join("ssr-manifest.json")).unwrap()).unwrap();
    let html = fs::read_to_string(dist.join("index.html")).unwrap();
    let page = &manifest["index.html"];
    let js = page["script"].as_str().unwrap();
    let [css] = page["css"].as_array().unwrap().as_slice() else {
        panic!("expected a bundled stylesheet");
    };
    let css = css.as_str().unwrap();
    for (url, ext) in [(js, ".js"), (css, ".css")] {
        assert!(url.starts_with("./assets/client-") && url.ends_with(ext));
        assert!(dist.join(url).is_file(), "missing {url}");
        assert!(html.contains(url), "{html}");
    }
}
