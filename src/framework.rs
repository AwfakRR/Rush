use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

pub fn solid(root: &Path) -> bool {
    fs::read(root.join("package.json"))
        .ok()
        .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
        .is_some_and(|package| {
            [
                "dependencies",
                "devDependencies",
                "peerDependencies",
                "optionalDependencies",
            ]
            .iter()
            .any(|section| package[*section]["solid-js"].is_string())
        })
}

pub fn handles(ext: &str, solid: bool) -> bool {
    matches!(ext, "vue" | "svelte") || solid && matches!(ext, "jsx" | "tsx")
}

#[derive(Deserialize)]
pub struct Component {
    pub code: String,
}

pub fn compile(root: &Path, source: &Path) -> Result<Component, String> {
    crate::css::run_node(root, "component", false, false, &[source])
}

fn collect_sources(
    root: &Path,
    dir: &Path,
    solid: bool,
    plain: &mut Vec<PathBuf>,
    components: &mut Vec<PathBuf>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with('.')
                && !matches!(name.as_ref(), "node_modules" | "dist" | "target")
            {
                collect_sources(root, &path, solid, plain, components);
            }
        } else if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
            if dir == root && crate::plugins::is_config(&path) {
                continue;
            }
            if handles(ext, solid) {
                components.push(path);
            } else if matches!(ext, "js" | "ts" | "jsx" | "tsx") {
                plain.push(path);
            }
        }
    }
}

pub fn prebundle(root: &Path, solid: bool) -> Result<(), String> {
    let mut plain = Vec::new();
    let mut components = Vec::new();
    collect_sources(root, root, solid, &mut plain, &mut components);
    if components.is_empty() && !solid {
        return crate::resolve::prebundle(root).map(|_| ());
    }

    let mut deps = BTreeSet::new();
    let mut scan = |source: &str, filename: &str| {
        deps.extend(
            crate::resolve::scan_imports(source, filename)
                .into_iter()
                .map(|(spec, _, _)| spec)
                .filter(|spec| crate::resolve::is_bare(spec)),
        );
    };
    for path in plain {
        if let Ok(source) = fs::read_to_string(&path) {
            scan(&source, &path.to_string_lossy());
        }
    }
    if !components.is_empty() {
        let paths: Vec<_> = components.iter().map(PathBuf::as_path).collect();
        let compiled: Vec<Component> =
            crate::css::run_node(root, "scan-components", false, false, &paths)?;
        for component in compiled {
            scan(&component.code, "component.ts");
        }
    }
    if !deps.is_empty() {
        let outdir = root.join("node_modules/.rush/deps");
        let paths: Vec<_> = std::iter::once(outdir.as_path())
            .chain(deps.iter().map(Path::new))
            .collect();
        crate::css::run_node::<serde_json::Value>(root, "framework-deps", false, false, &paths)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsx_is_only_handled_for_solid_projects() {
        let fixture = crate::test_util::TempDir::new();
        assert!(!solid(fixture.path()));
        assert!(!handles("tsx", false));
        assert!(handles("vue", false) && handles("svelte", false));
        fs::write(
            fixture.path().join("package.json"),
            r#"{ "devDependencies": { "solid-js": "^1.9.0" } }"#,
        )
        .unwrap();
        assert!(solid(fixture.path()));
        assert!(handles("tsx", true) && handles("jsx", true));
        assert!(!handles("ts", true));
    }
}
