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

#[test]
fn cli_build_splits_chunks_and_rewrites_pages() {
    let Some(esbuild) = esbuild() else {
        eprintln!("skipping: esbuild not found (run npm ci --prefix example)");
        return;
    };
    let f = Fixture::new();
    if esbuild.is_absolute() {
        std::fs::create_dir_all(f.path().join("node_modules/.bin")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&esbuild, f.path().join("node_modules/.bin/esbuild")).unwrap();
        #[cfg(windows)]
        std::fs::copy(&esbuild, f.path().join("node_modules/.bin/esbuild.exe")).unwrap();
    }
    check_split_pages(&f);
}

#[cfg(unix)]
#[test]
fn node_api_build_splits_chunks_and_rewrites_pages() {
    let modules = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("example/node_modules");
    if !modules.join("esbuild/package.json").is_file()
        || std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
    {
        eprintln!("skipping: Node.js and esbuild needed (npm ci --prefix example)");
        return;
    }
    let f = Fixture::new();
    std::os::unix::fs::symlink(modules, f.path().join("node_modules")).unwrap();
    check_split_pages(&f);
}

fn check_split_pages(f: &Fixture) {
    f.write(
        "index.html",
        r#"<a href="./about.html">About</a><script type="module" src="/src/home.js"></script>"#,
    )
    .write(
        "about.html",
        "<head></head><script type='module' src='./src/about.js'></script>",
    )
    .write("public/about.html", "must not replace the built page")
    .write(
        "src/home.js",
        "document.title = 'home-only'; window.load = () => import('./feature.js');",
    )
    .write(
        "src/about.js",
        "import './about.css'; document.title = 'about-only';",
    )
    .write("src/about.css", "body { display: grid; }")
    .write("src/feature.js", "export default 'lazy-feature-v1';")
    .write("public/report.txt", &"a".repeat(4096))
    .write("static.html", "<p>Standalone HTML page</p>");

    let build = || {
        let out = rush()
            .args(["build", "--root"])
            .arg(f.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    let report = build().replace("\x1b[90m", "").replace("\x1b[0m", "");
    let dist = f.path().join("dist");
    let home = std::fs::read_to_string(dist.join("index.html")).unwrap();
    let about = std::fs::read_to_string(dist.join("about.html")).unwrap();
    let home_js = hashed_url(&home, "./assets/home-", ".js");
    let about_js = hashed_url(&about, "./assets/about-", ".js");
    let about_css = hashed_url(&about, "./assets/about-", ".css");
    let read = |path: &str| std::fs::read_to_string(dist.join(path)).unwrap();
    assert!(home.contains("href=\"./about.html\""), "{home}");
    assert!(!home.contains(about_css), "{home}");
    let css = read(about_css);
    assert!(css.contains("display:grid"), "{css}");
    assert!(read(about_js).contains("about-only"));
    assert_eq!(read("static.html"), "<p>Standalone HTML page</p>");

    let home_bundle = read(home_js);
    assert!(home_bundle.contains("home-only"), "{home_bundle}");
    assert!(!home_bundle.contains("lazy-feature-v1"), "{home_bundle}");
    let chunk = hashed_url(&home_bundle, "./feature-", ".js");
    let chunk_path = format!("assets/{}", &chunk[2..]);
    assert!(read(&chunk_path).contains("lazy-feature-v1"));
    assert!(
        report.contains("Raw") && report.contains("Gzip"),
        "{report}"
    );
    for output in [
        "index.html",
        "about.html",
        "static.html",
        "report.txt",
        home_js,
        about_js,
        about_css,
        &chunk_path,
        &format!("{chunk_path}.map"),
    ] {
        let name = format!("dist/{}", output.trim_start_matches("./"));
        assert_eq!(
            report
                .lines()
                .filter(|line| {
                    line.trim_start()
                        .strip_prefix(&name)
                        .is_some_and(|rest| rest.starts_with(' '))
                })
                .count(),
            1,
            "expected one row for {name} in {report}",
        );
    }
    let sizes: Vec<_> = report
        .lines()
        .find(|line| line.contains("report.txt"))
        .unwrap()
        .split_whitespace()
        .skip(1)
        .collect();
    assert_eq!(&sizes[..2], ["4.00", "KiB"]);
    let gzip: f64 = sizes[2].parse().unwrap();
    assert!((0.02..=0.08).contains(&gzip), "{sizes:?}");
    assert_eq!(sizes[3], "KiB");

    f.write("src/feature.js", "export default 'lazy-feature-v2';");
    build();
    let rebuilt_home = read("index.html");
    let rebuilt_bundle = read(hashed_url(&rebuilt_home, "./assets/home-", ".js"));
    let rebuilt_chunk = hashed_url(&rebuilt_bundle, "./feature-", ".js");
    assert_ne!(rebuilt_chunk, chunk);
    assert!(!dist.join(chunk_path).exists());
    assert!(read(&format!("assets/{}", &rebuilt_chunk[2..])).contains("lazy-feature-v2"));
}

fn hashed_url<'a>(text: &'a str, prefix: &str, extension: &str) -> &'a str {
    let url = text
        .split(['\'', '"'])
        .find(|part| part.starts_with(prefix) && part.ends_with(extension))
        .unwrap_or_else(|| panic!("no {prefix}[hash]{extension} in {text}"));
    let hash = url
        .strip_prefix(prefix)
        .unwrap()
        .strip_suffix(extension)
        .unwrap();
    assert!(
        !hash.is_empty() && hash.bytes().all(|b| b.is_ascii_alphanumeric()),
        "{url}"
    );
    url
}
