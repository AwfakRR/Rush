use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};

const CONFIG_FILES: &[&str] = &[
    ".postcssrc",
    ".postcssrc.json",
    ".postcssrc.yaml",
    ".postcssrc.yml",
    ".postcssrc.js",
    ".postcssrc.cjs",
    ".postcssrc.mjs",
    ".postcssrc.ts",
    ".postcssrc.cts",
    ".postcssrc.mts",
    "postcss.config.js",
    "postcss.config.cjs",
    "postcss.config.mjs",
    "postcss.config.ts",
    "postcss.config.cts",
    "postcss.config.mts",
];

pub fn is_module(file: &Path) -> bool {
    file.file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with(".module.css"))
}

pub fn is_config(file: &Path) -> bool {
    file.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| CONFIG_FILES.contains(&name))
}

pub fn configured(root: &Path) -> bool {
    CONFIG_FILES.iter().any(|file| root.join(file).is_file())
        || std::fs::read_to_string(root.join("package.json"))
            .ok()
            .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
            .is_some_and(|json| json.get("postcss").is_some())
}

#[derive(Deserialize)]
pub struct Stylesheet {
    pub css: String,
    #[serde(default)]
    pub classes: BTreeMap<String, String>,
    #[serde(default)]
    pub exports: String,
}

impl Stylesheet {
    pub fn class_json(&self) -> String {
        serde_json::to_string(&self.classes).unwrap()
    }
}

pub fn process(root: &Path, file: &Path, production: bool) -> Result<Stylesheet, String> {
    let configured = configured(root);
    if configured || is_module(file) {
        run_node(root, "style", configured, production, &[file])
            .map_err(|error| format!("{}: {error}", file.display()))
    } else {
        let css = std::fs::read_to_string(file)
            .map_err(|error| format!("{}: {error}", file.display()))?;
        Ok(Stylesheet {
            css,
            classes: BTreeMap::new(),
            exports: String::new(),
        })
    }
}

pub fn run_node<T: serde::de::DeserializeOwned>(
    root: &Path,
    action: &str,
    configured: bool,
    production: bool,
    paths: &[&Path],
) -> Result<T, String> {
    let purpose = match action {
        "build" | "plugin-build" => "building the project",
        "style" => "CSS processing",
        _ => "framework compilation",
    };
    let output = Command::new("node")
        .current_dir(root)
        .env("RUSH_SOLID", crate::framework::solid(root).to_string())
        .env(
            "NODE_ENV",
            if production {
                "production"
            } else {
                "development"
            },
        )
        .args([
            "-e",
            concat!(
                include_str!("vite_plugins.cjs"), "\n",
                include_str!("framework.cjs"), "\n",
                include_str!("css.cjs"),
            ),
            "--",
            action,
            if configured { "true" } else { "false" },
        ])
        .args(paths)
        .output()
        .map_err(|error| format!("could not run Node.js for {purpose}: {error}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(stderr.trim().to_string());
    }
    if !stderr.trim().is_empty() {
        crate::log::warn(stderr.trim());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid Node.js output for {purpose}: {error}"))
}

#[derive(Default)]
pub struct StyleCache(Mutex<HashMap<PathBuf, Arc<Stylesheet>>>);

impl StyleCache {
    pub fn get(&self, root: &Path, file: &Path) -> Result<Arc<Stylesheet>, String> {
        let mut cache = self.0.lock().unwrap();
        if let Some(style) = cache.get(file) {
            return Ok(style.clone());
        }
        let style = Arc::new(process(root, file, false)?);
        cache.insert(file.to_owned(), style.clone());
        Ok(style)
    }

    pub fn invalidate(&self) {
        self.0.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    #[test]
    fn detects_project_postcss_configuration() {
        let dir = TempDir::new();
        assert!(!configured(dir.path()));
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"postcss":{"plugins":{}}}"#,
        )
        .unwrap();
        assert!(configured(dir.path()));
        std::fs::remove_file(dir.path().join("package.json")).unwrap();
        std::fs::write(dir.path().join(".postcssrc.mjs"), "export default {};").unwrap();
        assert!(configured(dir.path()));
    }
}
