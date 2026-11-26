use std::path::Path;

/// Load path aliases from tsconfig.json compilerOptions.paths.
/// ponytail: only supports the common "prefix/*": ["target/*"] pattern.
/// No complex multi-target resolution. serde_json::Value, no typed tsconfig struct.
pub fn load_aliases(root: &Path) -> Vec<(String, String)> {
    let mut aliases = Vec::new();

    let tsconfig_path = root.join("tsconfig.json");
    let raw = match std::fs::read_to_string(&tsconfig_path) {
        Ok(r) => r,
        Err(_) => return aliases,
    };

    let json: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return aliases,
    };

    let paths = match json.get("compilerOptions").and_then(|c| c.get("paths")) {
        Some(p) => p,
        None => return aliases,
    };

    if let Some(obj) = paths.as_object() {
        for (pattern, targets) in obj {
            // Strip trailing "/*" from pattern: "@/*" -> "@/"
            let prefix = pattern.trim_end_matches('*');
            // Get first target, strip trailing "/*": "./src/*" -> "./src/"
            if let Some(first_target) = targets.as_array().and_then(|a| a.first()).and_then(|v| v.as_str()) {
                let replacement = first_target.trim_end_matches('*');
                aliases.push((prefix.to_string(), replacement.to_string()));
            }
        }
    }

    // Sort by prefix length descending so longer prefixes match first
    aliases.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    aliases
}

/// Rewrite import specifiers matching alias prefixes to relative paths.
/// ponytail: simple string prefix match + replace in the import specifier.
pub fn resolve_aliases(source: &str, filename: &str, aliases: &[(String, String)]) -> String {
    if aliases.is_empty() {
        return source.to_string();
    }

    let imports = crate::resolve::scan_imports(source, filename);
    let mut result = source.to_string();

    // Process in reverse order so byte offsets remain valid
    for (spec, start, end) in imports.into_iter().rev() {
        for (prefix, replacement) in aliases {
            if spec.starts_with(prefix) {
                let rest = &spec[prefix.len()..];
                let resolved = format!("{replacement}{rest}");
                let rewritten = format!("\"{}\"", resolved);
                result.replace_range((start as usize)..(end as usize), &rewritten);
                break; // First matching alias wins
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_aliases_missing_tsconfig() {
        let aliases = load_aliases(Path::new("nonexistent_dir_12345"));
        assert!(aliases.is_empty());
    }

    #[test]
    fn test_resolve_aliases() {
        let aliases = vec![
            ("@/".to_string(), "./src/".to_string()),
            ("~/".to_string(), "./lib/".to_string()),
        ];

        let code = r#"import { Foo } from '@/components/Foo';
import { Bar } from '~/utils/bar';
import React from 'react';"#;

        let result = resolve_aliases(code, "app.tsx", &aliases);
        assert!(result.contains(r#"from "./src/components/Foo""#));
        assert!(result.contains(r#"from "./lib/utils/bar""#));
        // Bare import should be untouched
        assert!(result.contains(r#"from 'react'"#));
    }

    use crate::test_util::TempDir;

    #[test]
    fn test_load_aliases_from_tsconfig_sorted_longest_first() {
        let dir = TempDir::new();
        dir.write(
            "tsconfig.json",
            r#"{"compilerOptions":{"paths":{"@/*":["./src/*"],"@components/*":["./src/components/*","./fallback/*"],"~/*":["./lib/*"]}}}"#,
        );
        let aliases = load_aliases(dir.path());
        assert_eq!(aliases[0], ("@components/".to_string(), "./src/components/".to_string()));
        assert_eq!(aliases.len(), 3);
        assert!(aliases.contains(&("@/".to_string(), "./src/".to_string())));
        assert!(aliases.contains(&("~/".to_string(), "./lib/".to_string())));
    }

    #[test]
    fn test_load_aliases_tolerates_bad_tsconfig() {
        let dir = TempDir::new();
        dir.write("tsconfig.json", "{ invalid json");
        assert!(load_aliases(dir.path()).is_empty());
        dir.write("tsconfig.json", r#"{"compilerOptions":{"strict":true}}"#);
        assert!(load_aliases(dir.path()).is_empty());
    }

    #[test]
    fn test_resolve_aliases_longest_prefix_wins() {
        let dir = TempDir::new();
        dir.write(
            "tsconfig.json",
            r#"{"compilerOptions":{"paths":{"@/*":["./src/*"],"@components/*":["./ui/*"]}}}"#,
        );
        let aliases = load_aliases(dir.path());
        let out = resolve_aliases("import B from '@components/Button';\nimport u from '@/util';", "a.ts", &aliases);
        assert!(out.contains(r#"from "./ui/Button""#));
        assert!(out.contains(r#"from "./src/util""#));
    }

    #[test]
    fn test_resolve_aliases_no_aliases_is_identity() {
        let code = "import x from '@/x';";
        assert_eq!(resolve_aliases(code, "a.ts", &[]), code);
    }
}
