use std::{
    collections::HashMap,
    path::Path,
    time::Instant,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct EsbuildMetadata {
    inputs: HashMap<String, serde_json::Value>,
    outputs: HashMap<String, EsbuildOutput>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EsbuildOutput {
    entry_point: Option<String>,
    css_bundle: Option<String>,
}

fn bundle(root: &Path, entry: &Path, assets: &Path) -> Result<EsbuildMetadata, String> {
    let configured = crate::css::configured(root);
    if root.join("node_modules/esbuild/package.json").is_file() {
        return crate::css::run_node(root, "build", configured, true, &[entry, assets]);
    }
    if configured {
        return Err("PostCSS builds require the project's esbuild package. Run npm install -D esbuild.".into());
    }

    let esbuild = crate::resolve::find_esbuild(root)?;
    let metafile = assets.join(".rush-metafile.json");
    let output = std::process::Command::new(esbuild)
        .current_dir(root)
        .arg(entry)
        .args([
            "--bundle",
            "--minify",
            "--format=esm",
            "--sourcemap",
            "--target=es2020",
            &format!("--outdir={}", assets.display()),
            "--entry-names=[name]-[hash]",
            "--asset-names=[name]-[hash]",
            &format!("--metafile={}", metafile.display()),
        ])
        .output()
        .map_err(|e| format!("failed to execute esbuild: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("esbuild build failed:\n{stderr}"));
    }
    let json = std::fs::read(&metafile).map_err(|e| format!("failed to read esbuild metadata: {e}"))?;
    let _ = std::fs::remove_file(&metafile);
    let metadata: EsbuildMetadata = serde_json::from_slice(&json)
        .map_err(|e| format!("invalid esbuild metadata: {e}"))?;
    if metadata.inputs.keys().any(|input| input.ends_with(".module.css")) {
        return Err("CSS Modules builds require the project's esbuild package. Run npm install -D esbuild.".into());
    }
    Ok(metadata)
}

/// Extract entry script source from index.html `<script type="module" src="...">`
/// ponytail: simple string scan without full HTML AST parser.
pub fn extract_script_entry(html: &str) -> Option<String> {
    for tag in html.split("<script").skip(1) {
        let end = tag.find('>')?;
        let attrs = &tag[..end];
        if attrs.contains("type=\"module\"") || attrs.contains("type='module'") {
            if let Some(pos) = attrs.find("src=") {
                let rest = attrs[pos + 4..].trim_start();
                let quote = rest.chars().next()?;
                if quote == '"' || quote == '\'' {
                    let src = &rest[1..];
                    let end_quote = src.find(quote)?;
                    return Some(src[..end_quote].to_string());
                }
            }
        }
    }
    None
}

/// Extract linked stylesheets from index.html `<link rel="stylesheet" href="...">`
pub fn extract_css_links(html: &str) -> Vec<String> {
    let mut links = Vec::new();
    for tag in html.split("<link").skip(1) {
        if let Some(end) = tag.find('>') {
            let attrs = &tag[..end];
            if attrs.contains("rel=\"stylesheet\"") || attrs.contains("rel='stylesheet'") {
                if let Some(pos) = attrs.find("href=") {
                    let rest = attrs[pos + 5..].trim_start();
                    if let Some(quote) = rest.chars().next() {
                        if quote == '"' || quote == '\'' {
                            let href = &rest[1..];
                            if let Some(end_quote) = href.find(quote) {
                                links.push(href[..end_quote].to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    links
}

/// Run production build using esbuild.
/// Bundles, minifies, generates sourcemaps, and outputs to `dist/`.
pub fn run_build(root: &Path) -> Result<(), String> {
    let start = Instant::now();
    crate::log::info(&format!("building for production from {}", root.display()));

    let index_path = root.join("index.html");
    if !index_path.is_file() {
        return Err(format!("index.html not found in {}", root.display()));
    }

    let html_content = std::fs::read_to_string(&index_path)
        .map_err(|e| format!("failed to read index.html: {e}"))?;

    let script_entry = extract_script_entry(&html_content)
        .ok_or_else(|| "could not find <script type=\"module\" src=\"...\"> in index.html".to_string())?;

    let clean_script = script_entry.trim_start_matches('/');
    let entry_file = root.join(clean_script);
    if !entry_file.is_file() {
        return Err(format!("entry script file not found: {}", entry_file.display()));
    }

    // 1. Prepare dist/ directory
    let dist_dir = root.join("dist");
    let assets_dir = dist_dir.join("assets");
    let _ = std::fs::remove_dir_all(&dist_dir); // clean old build
    std::fs::create_dir_all(&assets_dir)
        .map_err(|e| format!("failed to create dist/assets directory: {e}"))?;

    let metadata = bundle(root, &entry_file, &assets_dir)?;
    let (built_js, entry_output) = metadata.outputs.iter()
        .find(|(file, output)| file.ends_with(".js") && output.entry_point.is_some())
        .ok_or_else(|| "no JavaScript bundle was generated by esbuild".to_string())?;
    let dist_url = |file: &str| {
        root.join(file).strip_prefix(&dist_dir)
            .map(|path| format!("./{}", path.to_string_lossy().replace('\\', "/")))
            .map_err(|_| format!("esbuild output outside dist: {file}"))
    };
    let js_url = dist_url(built_js)?;
    let mut built_files = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&assets_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(meta) = path.metadata() {
                let fname = path.file_name().unwrap().to_string_lossy().to_string();
                built_files.push((format!("assets/{fname}"), meta.len()));
            }
        }
    }

    // 5. Copy local stylesheets and update dist/index.html
    let mut rewritten_html = html_content.replace(
        &format!("src=\"{script_entry}\""),
        &format!("src=\"{js_url}\""),
    ).replace(
        &format!("src='{script_entry}'"),
        &format!("src='{js_url}'"),
    );

    let postcss = crate::css::configured(root);
    for css_href in extract_css_links(&html_content) {
        if !css_href.starts_with("http://") && !css_href.starts_with("https://") && !css_href.starts_with("//") {
            let clean_css = css_href.trim_start_matches('/');
            let src_css = root.join(clean_css);
            if src_css.is_file() {
                let dest_css = dist_dir.join(clean_css);
                if let Some(parent) = dest_css.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let written = if postcss || crate::css::is_module(&src_css) {
                    let style = crate::css::process(root, &src_css, true)?;
                    std::fs::write(&dest_css, style.css)
                } else {
                    std::fs::copy(&src_css, &dest_css).map(|_| ())
                };
                written.map_err(|e| format!("failed to write {}: {e}", dest_css.display()))?;
                if let Ok(meta) = dest_css.metadata() {
                    built_files.push((clean_css.to_string(), meta.len()));
                }
                rewritten_html = rewritten_html.replace(
                    &format!("href=\"{css_href}\""),
                    &format!("href=\"./{clean_css}\""),
                ).replace(
                    &format!("href='{css_href}'"),
                    &format!("href='./{clean_css}'"),
                );
            }
        }
    }

    if let Some(css_bundle) = &entry_output.css_bundle {
        let link = format!("<link rel=\"stylesheet\" href=\"{}\">\n", dist_url(css_bundle)?);
        if let Some(end) = rewritten_html.find("</head>") {
            rewritten_html.insert_str(end, &link);
        } else {
            rewritten_html.insert_str(0, &link);
        }
    }

    let dist_index = dist_dir.join("index.html");
    std::fs::write(&dist_index, &rewritten_html)
        .map_err(|e| format!("failed to write dist/index.html: {e}"))?;
    if let Ok(meta) = dist_index.metadata() {
        built_files.push(("index.html".to_string(), meta.len()));
    }

    // 6. Copy public directory if present
    let public_dir = root.join("public");
    if public_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&public_dir) {
            for entry in entries.flatten() {
                let src_path = entry.path();
                if src_path.is_file() {
                    let fname = src_path.file_name().unwrap();
                    let dest_path = dist_dir.join(fname);
                    let _ = std::fs::copy(&src_path, &dest_path);
                    if let Ok(meta) = dest_path.metadata() {
                        built_files.push((fname.to_string_lossy().to_string(), meta.len()));
                    }
                }
            }
        }
    }

    // 7. Output build summary
    let elapsed = start.elapsed().as_millis();
    println!();
    crate::log::ok(&format!("✓ built in {}ms into dist/", elapsed));
    println!();
    for (name, size) in built_files {
        let size_kb = size as f64 / 1024.0;
        println!("  {}dist/{:<28}{} {:>7.2} kB", crate::log::GRAY, name, crate::log::RESET, size_kb);
    }
    println!();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_script_entry() {
        let html1 = r#"<html><head><script type="module" src="/src/main.tsx"></script></head></html>"#;
        assert_eq!(extract_script_entry(html1), Some("/src/main.tsx".to_string()));

        let html2 = r#"<html><body><script type='module' src="./app.js"></script></body></html>"#;
        assert_eq!(extract_script_entry(html2), Some("./app.js".to_string()));

        let html_none = r#"<html><body><script src="regular.js"></script></body></html>"#;
        assert_eq!(extract_script_entry(html_none), None);
    }

    #[test]
    fn test_extract_css_links() {
        let html = r#"<html><head><link rel="stylesheet" href="/style.css" /><link rel="icon" href="/favicon.ico" /></head></html>"#;
        let links = extract_css_links(html);
        assert_eq!(links, vec!["/style.css".to_string()]);
    }

    use crate::test_util::TempDir;

    #[test]
    fn test_extract_script_entry_skips_non_module_scripts() {
        let html = r#"<script src="/legacy.js"></script><script async type="module" src='/src/main.ts'></script>"#;
        assert_eq!(extract_script_entry(html), Some("/src/main.ts".to_string()));
    }

    #[test]
    fn test_extract_css_links_multiple_and_single_quotes() {
        let html = r#"<link rel='stylesheet' href='/a.css'><link href="/b.css"><link rel="stylesheet" href="https://cdn/x.css">"#;
        assert_eq!(extract_css_links(html), vec!["/a.css".to_string(), "https://cdn/x.css".to_string()]);
    }

    #[test]
    fn test_run_build_errors() {
        let dir = TempDir::new();
        assert!(run_build(dir.path()).unwrap_err().contains("index.html not found"));

        dir.write("index.html", "<html><body></body></html>");
        assert!(run_build(dir.path()).unwrap_err().contains("<script type=\"module\""));

        dir.write("index.html", r#"<script type="module" src="/missing.js"></script>"#);
        assert!(run_build(dir.path()).unwrap_err().contains("entry script file not found"));
    }
}
