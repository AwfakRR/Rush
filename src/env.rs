use std::collections::HashMap;
use std::path::Path;

/// Load environment variables from `.env` file.
/// ponytail: hand-rolled parser (~20 lines). No dotenv crate — it's trivial string splitting.
/// Only exposes RUSH_-prefixed vars (prevents leaking DB passwords etc.) plus DEV/MODE builtins.
pub fn load_env(root: &Path) -> HashMap<String, String> {
    let mut env = HashMap::new();

    // Built-in env vars (always available)
    env.insert("import.meta.env.DEV".to_string(), "true".to_string());
    env.insert("import.meta.env.PROD".to_string(), "false".to_string());
    env.insert("import.meta.env.MODE".to_string(), "\"development\"".to_string());

    // Read .env file if it exists
    let env_path = root.join(".env");
    if let Ok(content) = std::fs::read_to_string(&env_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            // Skip comments and empty lines
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = trimmed.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                // ponytail: only expose RUSH_-prefixed vars for security
                if key.starts_with("RUSH_") {
                    // Strip surrounding quotes if present
                    let clean_value = value
                        .strip_prefix('"').and_then(|v| v.strip_suffix('"'))
                        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                        .unwrap_or(value);
                    env.insert(
                        format!("import.meta.env.{key}"),
                        format!("\"{}\"", clean_value),
                    );
                }
            }
        }
    }

    env
}

/// Replace `import.meta.env.*` references in transpiled JS with their values.
/// ponytail: naive str::replace loop. No AST-level define transform.
/// Works for 99% of cases. Add AST transform if computed access on import.meta.env is needed.
pub fn replace_env_vars(code: &str, env: &HashMap<String, String>) -> String {
    let mut result = code.to_string();
    for (key, value) in env {
        result = result.replace(key, value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_env_builtins() {
        // Even without .env file, builtins should be present
        let env = load_env(Path::new("nonexistent_dir_12345"));
        assert_eq!(env.get("import.meta.env.DEV"), Some(&"true".to_string()));
        assert_eq!(env.get("import.meta.env.MODE"), Some(&"\"development\"".to_string()));
    }

    #[test]
    fn test_replace_env_vars() {
        let mut env = HashMap::new();
        env.insert("import.meta.env.DEV".to_string(), "true".to_string());
        env.insert("import.meta.env.RUSH_API".to_string(), "\"http://localhost:8080\"".to_string());

        let code = "if (import.meta.env.DEV) { fetch(import.meta.env.RUSH_API); }";
        let result = replace_env_vars(code, &env);

        assert!(result.contains("if (true)"));
        assert!(result.contains("fetch(\"http://localhost:8080\")"));
        assert!(!result.contains("import.meta.env"));
    }
}
