use oxc::{
    allocator::Allocator,
    ast::ast::ModuleDeclaration,
    parser::Parser,
    span::SourceType,
};
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
}
