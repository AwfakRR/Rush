mod common;

use common::{rush, Fixture};
use std::{fs, path::PathBuf, process::Command};

fn with_esbuild(fixture: &Fixture) -> bool {
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("example/node_modules/.bin/esbuild");
    if local.is_file() {
        let bin = fixture.path().join("node_modules/.bin");
        fs::create_dir_all(&bin).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(local, bin.join("esbuild")).unwrap();
        #[cfg(windows)]
        fs::copy(local, bin.join("esbuild.exe")).unwrap();
        return true;
    }
    if Command::new("esbuild").arg("--version").output().is_ok() {
        return true;
    }
    eprintln!("skipping: esbuild not found (run npm ci --prefix example)");
    false
}

#[test]
fn builds_a_dual_format_library_with_external_dependencies_and_declarations() {
    let f = Fixture::new();
    if !with_esbuild(&f) {
        return;
    }
    f.write(
        "package.json",
        r#"{
            "type": "module",
            "source": "src/public.ts",
            "dependencies": {"fake-dep": "1.0.0"},
            "devDependencies": {"dev-only": "1.0.0"}
        }"#,
    )
    .write("src/public.ts", "export const fromManifest: number = 0;")
    .write(
        "src/index.ts",
        r#"import { bonus } from "fake-dep/bonus";
import { double } from "dev-only";
import { sep } from "node:path";
export { add } from "./nested";
export type { Input } from "./nested/input.js";
export function total(value: number): number { return double(value) + bonus; }
export function separator(): string { return sep; }"#,
    )
    .write(
        "src/nested/index.ts",
        r#"
export function add(a: number, b: number): number { return a + b; }
export type { Input } from "./input.js";
"#,
    )
    .write(
        "src/nested/input.ts",
        "export interface Input { amount: number; }",
    )
    .write(
        "node_modules/fake-dep/package.json",
        r#"{"exports":{"./bonus":"./bonus.cjs"}}"#,
    )
    .write(
        "node_modules/fake-dep/bonus.cjs",
        "exports.bonus = 2; // never-inline-this-dependency",
    )
    .write(
        "node_modules/dev-only/package.json",
        r#"{"main":"./index.cjs"}"#,
    )
    .write(
        "node_modules/dev-only/index.cjs",
        "exports.double = value => value * 2;",
    )
    .write("dist/stale.txt", "old build output");
    let out = rush()
        .args(["build", "--lib", "--entry", "src/index.ts", "--root"])
        .arg(f.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let dist = f.path().join("dist");
    let read = |name: &str| fs::read_to_string(dist.join(name)).unwrap();
    let esm = read("index.mjs");
    let cjs = read("index.cjs");
    assert!(esm.contains(r#"from "fake-dep/bonus""#), "{esm}");
    assert!(cjs.contains(r#"require("fake-dep/bonus")"#), "{cjs}");
    assert!(esm.contains(r#"from "node:path""#), "{esm}");
    assert!(cjs.contains(r#"require("node:path")"#), "{cjs}");
    for bundle in [&esm, &cjs] {
        assert!(!bundle.contains("exports.bonus = 2"), "{bundle}");
        assert!(bundle.contains("value * 2"), "{bundle}");
    }
    for file in ["index.mjs.map", "index.cjs.map"] {
        assert!(dist.join(file).is_file());
    }
    assert!(!dist.join("stale.txt").exists());

    for (types, specifier) in [("d.ts", "js"), ("d.mts", "mjs"), ("d.cts", "cjs")] {
        let entry = read(&format!("index.{types}"));
        assert!(
            entry.contains("function total(value: number): number;"),
            "{entry}"
        );
        assert!(
            entry.contains(&format!("./nested/index.{specifier}")),
            "{entry}"
        );
        assert!(
            entry.contains(&format!("./nested/input.{specifier}")),
            "{entry}"
        );
        let nested = read(&format!("nested/index.{types}"));
        assert!(
            nested.contains("function add(a: number, b: number): number;"),
            "{nested}"
        );
        assert!(nested.contains(&format!("./input.{specifier}")), "{nested}");
        assert!(read(&format!("nested/input.{types}")).contains("interface Input"));
    }

    if Command::new("node").arg("--version").output().is_ok() {
        for (kind, js) in [
            (
                "module",
                "import { total, add } from './dist/index.mjs'; console.log(total(add(18, 2)));",
            ),
            (
                "commonjs",
                "const {total, add} = require('./dist/index.cjs'); console.log(total(add(18, 2)));",
            ),
        ] {
            let output = Command::new("node")
                .current_dir(f.path())
                .args(["--input-type", kind, "-e", js])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
        }
    }
}

#[test]
fn javascript_entry_skips_declarations() {
    let f = Fixture::new();
    if !with_esbuild(&f) {
        return;
    }
    f.write("package.json", r#"{"source":"src/main.js"}"#)
        .write("src/main.js", "export const answer = 42;");
    let out = rush()
        .args(["build", "--lib", "--root"])
        .arg(f.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let combined = [&out.stdout[..], &out.stderr[..]].concat();
    assert!(String::from_utf8_lossy(&combined).contains("skipping TypeScript declarations"));
    for file in ["main.mjs", "main.cjs"] {
        assert!(f.path().join("dist").join(file).is_file());
    }
    assert!(!f.path().join("dist/main.d.ts").exists());
}

#[test]
fn build_reports_a_missing_entry() {
    let f = Fixture::new();
    let out = rush()
        .args(["build", "--lib", "--root"])
        .arg(f.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no library entry found"));
}
