use oxc::{
    allocator::Allocator,
    ast::ast::ModuleDeclaration,
    parser::Parser,
    span::SourceType,
};
use serde::{Deserialize, Serialize};
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Check if an import specifier is a bare package name (not relative, absolute, or external URL).
pub fn is_bare(spec: &str) -> bool {
    // ponytail: 4 simple checks, no url crate, no regex
    !spec.starts_with('.')
        && !spec.starts_with('/')
        && !spec.starts_with("http://")
        && !spec.starts_with("https://")
}

/// URL path where pre-bundled dependency is served.
fn dep_url(deps_dir: &str, specifier: &str) -> String {
    // ponytail: slashes in scoped packages (@foo/bar) become double underscores
    format!("{}/{}.js", deps_dir.trim_end_matches('/'), specifier.replace('/', "__"))
}

/// Scan JS/TS source code for import and export specifiers.
/// Returns Vec<(specifier_string, span_start, span_end)>.
pub fn scan_imports(source: &str, filename: &str) -> Vec<(String, u32, u32)> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(Path::new(filename)).unwrap_or_default();
    let parser_ret = Parser::new(&allocator, source, source_type).parse();
    let mut imports = Vec::new();

    // ponytail: walk top-level statements, no full AST visitor pass needed
    for stmt in &parser_ret.program.body {
        if let Some(decl) = stmt.as_module_declaration() {
            match decl {
                ModuleDeclaration::ImportDeclaration(import_decl) => {
                    let spec = import_decl.source.value.to_string();
                    imports.push((spec, import_decl.source.span.start, import_decl.source.span.end));
                }
                ModuleDeclaration::ExportAllDeclaration(export_decl) => {
                    let spec = export_decl.source.value.to_string();
                    imports.push((spec, export_decl.source.span.start, export_decl.source.span.end));
                }
                _ => {}
            }
        }
    }

    imports
}

/// Rewrite bare imports in JS source to point at pre-bundled dependencies.
/// ponytail: modifies string directly using byte offsets in reverse order.
/// No AST-to-string round-trip, no codegen dependency for this step.
pub fn rewrite_bare_imports(source: &str, filename: &str, deps_dir: &str) -> String {
    let imports = scan_imports(source, filename);
    let mut result = source.to_string();

    // Process in reverse order so byte offsets remain valid after string replacement
    for (spec, start, end) in imports.into_iter().rev() {
        if is_bare(&spec) {
            let target_url = dep_url(deps_dir, &spec);
            let rewritten = format!("\"{}\"", target_url);
            result.replace_range((start as usize)..(end as usize), &rewritten);
        }
    }

    result
}

/// Resolve a bare package name to its entry file in node_modules.
/// ponytail: reads package.json with serde_json::Value, no typed struct needed.
/// Handles exports.".".import, exports.".".default, module, main, index.js.
#[allow(dead_code)]
pub fn resolve_package_entry(root: &Path, package_name: &str) -> Option<PathBuf> {
    let pkg_dir = root.join("node_modules").join(package_name);
    let pkg_json_path = pkg_dir.join("package.json");
    let raw = std::fs::read_to_string(&pkg_json_path).ok()?;
    let pkg: serde_json::Value = serde_json::from_str(&raw).ok()?;

    // 1. Check "exports" field
    if let Some(exports) = pkg.get("exports") {
        if let Some(dot) = exports.get(".") {
            for key in ["import", "default", "browser"] {
                if let Some(p) = dot.get(key).and_then(|v| v.as_str()) {
                    return Some(pkg_dir.join(p));
                }
            }
            if let Some(p) = dot.as_str() {
                return Some(pkg_dir.join(p));
            }
        }
        if let Some(p) = exports.as_str() {
            return Some(pkg_dir.join(p));
        }
    }

    // 2. Check "module" field (standard for modern ESM builds)
    if let Some(p) = pkg.get("module").and_then(|v| v.as_str()) {
        return Some(pkg_dir.join(p));
    }

    // 3. Check "main" field
    if let Some(p) = pkg.get("main").and_then(|v| v.as_str()) {
        return Some(pkg_dir.join(p));
    }

    // 4. Fallback to index.js
    let fallback = pkg_dir.join("index.js");
    if fallback.exists() {
        return Some(fallback);
    }

    None
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DepCache {
    pub hash: String,
    pub deps: Vec<String>,
}

/// Compute a fast hash over all lockfiles and package.json to detect dependency changes.
/// ponytail: DefaultHasher, no crypto crate needed for cache invalidation.
pub fn lockfile_hash(root: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    for name in ["package-lock.json", "pnpm-lock.yaml", "yarn.lock", "bun.lockb", "package.json"] {
        let p = root.join(name);
        if let Ok(bytes) = std::fs::read(&p) {
            bytes.hash(&mut hasher);
        }
    }
    format!("{:016x}", hasher.finish())
}

/// Scan a directory recursively for bare imports across all source files.
/// ponytail: simple std::fs::read_dir recursion, skips noise dirs without heavy walkdir/glob crates.
pub fn scan_dir_for_bare_imports(root: &Path, dir: &Path, bare: &mut HashSet<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "node_modules" || name == ".git" || name == "target" || name.starts_with('.') {
                continue;
            }
            scan_dir_for_bare_imports(root, &path, bare);
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "js" | "ts" | "jsx" | "tsx") {
                if let Ok(source) = std::fs::read_to_string(&path) {
                    let fname = path.file_name().unwrap().to_string_lossy().to_string();
                    for (spec, _, _) in scan_imports(&source, &fname) {
                        if is_bare(&spec) {
                            bare.insert(spec);
                        }
                    }
                }
            }
        }
    }
}

/// Find esbuild executable in local node_modules or system PATH.
pub fn find_esbuild(root: &Path) -> Result<String, String> {
    // 1. Check local node_modules/.bin
    for bin in ["esbuild.cmd", "esbuild.exe", "esbuild"] {
        let local = root.join("node_modules").join(".bin").join(bin);
        if local.exists() {
            return Ok(local.to_string_lossy().to_string());
        }
    }

    // 2. Check if esbuild is available on system PATH
    if std::process::Command::new("esbuild").arg("--version").output().is_ok() {
        return Ok("esbuild".to_string());
    }

    Err("esbuild not found. Install it with: npm install -D esbuild".to_string())
}

/// Pre-bundle discovered node_modules dependencies into ESM files using esbuild.
/// ponytail: shells out to esbuild binary — don't reinvent a JavaScript bundler.
pub fn prebundle(root: &Path) -> Result<Option<DepCache>, String> {
    let deps_dir = root.join("node_modules").join(".rush").join("deps");
    let cache_file = deps_dir.join("_cache.json");
    let current_hash = lockfile_hash(root);

    // 1. Check if existing cache is still fresh
    if let Ok(raw) = std::fs::read_to_string(&cache_file) {
        if let Ok(cache) = serde_json::from_str::<DepCache>(&raw) {
            if cache.hash == current_hash {
                return Ok(Some(cache));
            }
        }
    }

    // 2. Scan all project files for bare imports
    let mut bare_set = HashSet::new();
    scan_dir_for_bare_imports(root, root, &mut bare_set);

    if bare_set.is_empty() {
        return Ok(None);
    }

    // 3. Resolve entry points in node_modules
    let mut entries = Vec::new();
    for pkg in &bare_set {
        let mut entry_opt = resolve_package_entry(root, pkg);
        if entry_opt.is_none() && root.join("package.json").exists() {
            // Phase 14: Auto-install missing dependency if package.json exists in root
            if let Ok(()) = auto_install_package(root, pkg) {
                entry_opt = resolve_package_entry(root, pkg);
            }
        }

        if let Some(entry_file) = entry_opt {
            let out_name = pkg.replace('/', "__");
            entries.push((out_name, entry_file));
        } else {
            crate::log::warn(&format!("could not resolve entry for package '{pkg}'"));
        }
    }

    if entries.is_empty() {
        return Ok(None);
    }

    // 4. Shell out to esbuild
    let esbuild_bin = find_esbuild(root)?;
    std::fs::create_dir_all(&deps_dir).map_err(|e| e.to_string())?;

    let mut cmd = std::process::Command::new(&esbuild_bin);
    cmd.current_dir(root);

    // Pass entry points: <out_name>=<entry_path>
    for (out_name, entry_path) in &entries {
        cmd.arg(format!("{}={}", out_name, entry_path.display()));
    }

    cmd.args([
        "--bundle",
        "--format=esm",
        "--splitting",
        "--platform=browser",
        "--target=es2020",
        &format!("--outdir={}", deps_dir.display()),
        "--sourcemap",
    ]);

    let output = cmd.output().map_err(|e| format!("Failed to run esbuild: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("esbuild bundling error:\n{stderr}"));
    }

    // 5. Save cache metadata
    let mut deps_list: Vec<String> = bare_set.into_iter().collect();
    deps_list.sort();
    let cache = DepCache {
        hash: current_hash,
        deps: deps_list,
    };

    if let Ok(json) = serde_json::to_string_pretty(&cache) {
        let _ = std::fs::write(&cache_file, json);
    }

    crate::log::ok(&format!("pre-bundled {} dependencies into {}", cache.deps.len(), deps_dir.display()));
    Ok(Some(cache))
}

/// Detect the active package manager by checking for lockfiles in the root.
pub fn detect_package_manager(root: &Path) -> &'static str {
    if root.join("pnpm-lock.yaml").exists() {
        "pnpm"
    } else if root.join("yarn.lock").exists() {
        "yarn"
    } else if root.join("bun.lockb").exists() || root.join("bun.lock").exists() {
        "bun"
    } else {
        "npm"
    }
}

/// Auto-install a missing dependency package.
/// ponytail: invoke package manager via std::process::Command. No custom npm client.
pub fn auto_install_package(root: &Path, pkg: &str) -> Result<(), String> {
    let pm = detect_package_manager(root);
    crate::log::info(&format!("auto-installing missing dependency '{pkg}' via {pm}..."));

    let (cmd_name, args) = match pm {
        "pnpm" => {
            let cmd = if cfg!(windows) { "pnpm.cmd" } else { "pnpm" };
            (cmd, vec!["add", pkg])
        }
        "yarn" => {
            let cmd = if cfg!(windows) { "yarn.cmd" } else { "yarn" };
            (cmd, vec!["add", pkg])
        }
        "bun" => {
            let cmd = if cfg!(windows) { "bun.exe" } else { "bun" };
            (cmd, vec!["add", pkg])
        }
        _ => {
            let cmd = if cfg!(windows) { "npm.cmd" } else { "npm" };
            (cmd, vec!["install", pkg])
        }
    };

    let status = std::process::Command::new(cmd_name)
        .args(&args)
        .current_dir(root)
        .status()
        .map_err(|e| format!("failed to execute {cmd_name}: {e}"))?;

    if status.success() {
        crate::log::ok(&format!("installed '{pkg}' successfully"));
        Ok(())
    } else {
        Err(format!("{cmd_name} exited with status: {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_bare() {
        assert!(is_bare("react"));
        assert!(is_bare("@tanstack/react-query"));
        assert!(is_bare("lodash-es"));
        assert!(!is_bare("./utils"));
        assert!(!is_bare("../components/Button"));
        assert!(!is_bare("/assets/logo.png"));
        assert!(!is_bare("https://esm.sh/react"));
        assert!(!is_bare("http://localhost:3000/app.js"));
    }

    #[test]
    fn test_rewrite_bare_imports() {
        let code = r#"import React, { useState } from 'react';
import { Button } from './components/Button.js';
export * from 'lodash-es';"#;

        let rewritten = rewrite_bare_imports(code, "app.js", "/node_modules/.rush/deps");

        assert!(rewritten.contains(r#"from "/node_modules/.rush/deps/react.js";"#));
        assert!(rewritten.contains(r#"from "/node_modules/.rush/deps/lodash-es.js";"#));
        // Relative imports must remain untouched
        assert!(rewritten.contains(r#"from './components/Button.js';"#));
    }

    #[test]
    fn test_scoped_package_rewrite() {
        let code = r#"import { useQuery } from '@tanstack/react-query';"#;
        let rewritten = rewrite_bare_imports(code, "app.js", "/node_modules/.rush/deps");
        assert!(rewritten.contains(r#"from "/node_modules/.rush/deps/@tanstack__react-query.js";"#));
    }

    #[test]
    fn test_lockfile_hash() {
        let hash1 = lockfile_hash(Path::new("."));
        let hash2 = lockfile_hash(Path::new("."));
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 16);
    }

    #[test]
    fn test_scan_dir_for_bare_imports() {
        let mut bare = HashSet::new();
        scan_dir_for_bare_imports(Path::new("example"), Path::new("example"), &mut bare);
        assert!(bare.contains("canvas-confetti"));
    }

    #[test]
    fn test_detect_package_manager() {
        // In this project root, there is Cargo.lock, not yarn or pnpm, so defaults to npm
        assert_eq!(detect_package_manager(Path::new(".")), "npm");
    }
}
