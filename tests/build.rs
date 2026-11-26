//! `rush build` end to end. Needs esbuild: example/node_modules (npm install in example/) or PATH.
mod common;

use common::{rush, Fixture};
use std::path::PathBuf;

fn esbuild() -> Option<PathBuf> {
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("example/node_modules/.bin/esbuild");
    if local.exists() {
        return Some(local);
    }
    std::process::Command::new("esbuild")
        .arg("--version")
        .output()
        .ok()
        .map(|_| PathBuf::from("esbuild"))
}

#[test]
fn build_bundles_entry_css_and_public() {
    let Some(esbuild) = esbuild() else {
        eprintln!("skipping: esbuild not found (run npm install in example/)");
        return;
    };

    let f = Fixture::new();
    f.write(
        "index.html",
        "<html><head><link rel=\"stylesheet\" href=\"/css/style.css\"><link rel=\"stylesheet\" href=\"https://cdn.example/x.css\"></head><body><script type=\"module\" src=\"/src/main.ts\"></script></body></html>",
    )
    .write("src/main.ts", "import { add } from './math';\nconst total: number = add(40, 2);\ndocument.title = `total-${total}`;\n")
    .write("src/math.ts", "export function add(a: number, b: number): number { return a + b; }\n")
    .write("css/style.css", "body { margin: 0; }\n")
    .write("public/robots.txt", "User-agent: *\n")
    .write("dist/stale.txt", "old build output");

    if esbuild.is_absolute() {
        std::fs::create_dir_all(f.path().join("node_modules/.bin")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&esbuild, f.path().join("node_modules/.bin/esbuild")).unwrap();
        #[cfg(windows)]
        std::fs::copy(&esbuild, f.path().join("node_modules/.bin/esbuild.exe")).unwrap();
    }

    let out = rush()
        .args(["build", "--root", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let dist = f.path().join("dist");
    assert!(!dist.join("stale.txt").exists(), "dist/ was not cleaned");

    let html = std::fs::read_to_string(dist.join("index.html")).unwrap();
    let js_name = std::fs::read_dir(dist.join("assets"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .find(|n| n.starts_with("main-") && n.ends_with(".js"))
        .expect("hashed main-*.js bundle");
    assert!(
        html.contains(&format!("src=\"./assets/{js_name}\"")),
        "{html}"
    );
    assert!(html.contains("href=\"./css/style.css\""), "{html}");
    assert!(
        html.contains("href=\"https://cdn.example/x.css\""),
        "{html}"
    );
    assert!(
        !html.contains("__hmr"),
        "production HTML must not include the HMR client"
    );

    let js = std::fs::read_to_string(dist.join("assets").join(&js_name)).unwrap();
    assert!(js.contains("total-"), "{js}");
    assert!(!js.contains(": number"), "types not stripped: {js}");
    assert!(dist.join("assets").join(format!("{js_name}.map")).exists());

    assert_eq!(
        std::fs::read_to_string(dist.join("css/style.css")).unwrap(),
        "body { margin: 0; }\n"
    );
    assert_eq!(
        std::fs::read_to_string(dist.join("robots.txt")).unwrap(),
        "User-agent: *\n"
    );
}

#[test]
fn build_fails_without_index_html() {
    let f = Fixture::new();
    let out = rush()
        .args(["build", "--root", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("index.html not found"));
}
