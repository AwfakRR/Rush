use std::path::{Path, PathBuf};

pub fn resolve_entry(root: &Path, entry: &Path) -> Result<PathBuf, String> {
    root.join(entry)
        .canonicalize()
        .ok()
        .filter(|file| file.is_file() && file.starts_with(root))
        .ok_or_else(|| {
            format!(
                "SSR entry must be a file inside the project: {}",
                entry.display()
            )
        })
}

pub fn bundle(root: &Path, entry: &Path, production: bool) -> Result<String, String> {
    let config = crate::plugins::find_config(root);
    let paths: Vec<_> = std::iter::once(entry).chain(config.as_deref()).collect();
    crate::css::run_node(
        root,
        "ssr",
        crate::css::configured(root),
        production,
        &paths,
    )
}
