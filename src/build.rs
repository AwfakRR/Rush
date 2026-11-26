use flate2::{read::GzEncoder, Compression};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs, io,
    path::{Path, PathBuf},
    time::Instant,
};

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

#[derive(Serialize)]
struct ClientAssets {
    script: String,
    css: Vec<String>,
}

fn bundle(
    root: &Path,
    entries: &BTreeSet<PathBuf>,
    assets: &Path,
) -> Result<EsbuildMetadata, String> {
    let configured = crate::css::configured(root);
    let config = crate::plugins::find_config(root);
    if root.join("node_modules/esbuild/package.json").is_file() {
        let paths: Vec<_> = std::iter::once(assets)
            .chain(config.iter().map(PathBuf::as_path))
            .chain(entries.iter().map(PathBuf::as_path))
            .collect();
        let action = if config.is_some() { "plugin-build" } else { "build" };
        return crate::css::run_node(root, action, configured, true, &paths);
    }
    if config.is_some() {
        return Err("plugin builds require the project's esbuild package. Run npm install -D esbuild.".into());
    }
    if configured {
        return Err(
            "PostCSS builds require the project's esbuild package. Run npm install -D esbuild."
                .into(),
        );
    }

    let esbuild = crate::resolve::find_esbuild(root)?;
    let metafile = assets.join(".rush-metafile.json");
    let output = std::process::Command::new(esbuild)
        .current_dir(root)
        .args(entries)
        .args([
            "--bundle",
            "--splitting",
            "--minify",
            "--format=esm",
            "--sourcemap",
            "--target=es2020",
            &format!("--outdir={}", assets.display()),
            "--entry-names=[name]-[hash]",
            "--chunk-names=[name]-[hash]",
            "--asset-names=[name]-[hash]",
            &format!("--metafile={}", metafile.display()),
        ])
        .output()
        .map_err(|e| format!("failed to execute esbuild: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("esbuild build failed:\n{stderr}"));
    }
    let json =
        std::fs::read(&metafile).map_err(|e| format!("failed to read esbuild metadata: {e}"))?;
    let _ = std::fs::remove_file(&metafile);
    let metadata: EsbuildMetadata =
        serde_json::from_slice(&json).map_err(|e| format!("invalid esbuild metadata: {e}"))?;
    if metadata
        .inputs
        .keys()
        .any(|input| input.ends_with(".module.css"))
    {
        return Err(
            "CSS Modules builds require the project's esbuild package. Run npm install -D esbuild."
                .into(),
        );
    }
    Ok(metadata)
}

/// Extract entry script source from an HTML page's `<script type="module" src="...">`
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

/// Extract linked stylesheets from an HTML page's `<link rel="stylesheet" href="...">`
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
pub fn run_build(root: &Path, ssr_entry: Option<&Path>) -> Result<(), String> {
    let start = Instant::now();
    crate::log::info(&format!("building for production from {}", root.display()));

    let read_dir_error = |e: io::Error| format!("failed to read {}: {e}", root.display());
    let mut pages = Vec::new();
    let mut entries = BTreeSet::new();
    for item in fs::read_dir(root).map_err(read_dir_error)? {
        let path = item.map_err(read_dir_error)?.path();
        if path.extension().is_none_or(|ext| ext != "html") || !path.is_file() {
            continue;
        }
        let html = fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        let script = extract_script_entry(&html)
            .map(|src| {
                let file = root.join(src.trim_start_matches('/'));
                let entry = file
                    .canonicalize()
                    .ok()
                    .filter(|path| path.is_file())
                    .ok_or_else(|| format!("entry script file not found: {}", file.display()))?;
                entries.insert(entry.clone());
                Ok::<_, String>((src, entry))
            })
            .transpose()?;
        pages.push((path, html, script));
    }
    if pages.is_empty() {
        return Err(format!(
            "index.html not found (no root HTML pages) in {}",
            root.display()
        ));
    }
    if entries.is_empty() {
        return Err("could not find <script type=\"module\" src=\"...\"> in any HTML page".into());
    }

    let server_bundle = ssr_entry
        .map(|entry| crate::ssr::resolve_entry(root, entry))
        .transpose()?
        .map(|entry| crate::ssr::bundle(root, &entry, true))
        .transpose()?;
    let dist_dir = root.join("dist");
    let assets_dir = dist_dir.join("assets");
    let _ = fs::remove_dir_all(&dist_dir);
    fs::create_dir_all(&assets_dir)
        .map_err(|e| format!("failed to create dist/assets directory: {e}"))?;

    let metadata = bundle(root, &entries, &assets_dir)?;
    let outputs: HashMap<_, _> = metadata
        .outputs
        .iter()
        .filter_map(|(file, output)| {
            if !file.ends_with(".js") {
                return None;
            }
            let entry = root
                .join(output.entry_point.as_ref()?)
                .canonicalize()
                .ok()?;
            entries.contains(&entry).then_some((entry, (file, output)))
        })
        .collect();
    let dist_url = |file: &str| {
        root.join(file)
            .strip_prefix(&dist_dir)
            .map(|path| format!("./{}", path.to_string_lossy().replace('\\', "/")))
            .map_err(|_| format!("esbuild output outside dist: {file}"))
    };

    let postcss = crate::css::configured(root);
    let mut manifest = BTreeMap::new();
    for (page, html, script) in pages {
        let links = extract_css_links(&html);
        let mut rewritten = html;
        let mut css_urls = Vec::new();
        for href in links {
            if href.starts_with("http://") || href.starts_with("https://") || href.starts_with("//")
            {
                continue;
            }
            let clean = href.trim_start_matches('/');
            let src = root.join(clean);
            if !src.is_file() {
                continue;
            }
            let dest = dist_dir.join(clean);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
            }
            let written = if postcss || crate::css::is_module(&src) {
                let style = crate::css::process(root, &src, true)?;
                fs::write(&dest, style.css)
            } else {
                fs::copy(&src, &dest).map(|_| ())
            };
            written.map_err(|e| format!("failed to write {}: {e}", dest.display()))?;
            let url = format!("./{clean}");
            rewritten = rewrite_attr(&rewritten, "href", &href, &url);
            css_urls.push(url);
        }
        if let Some((src, entry)) = script {
            let (js, output) = outputs.get(&entry).ok_or_else(|| {
                format!("no JavaScript bundle was generated for {}", entry.display())
            })?;
            let script = dist_url(js)?;
            rewritten = rewrite_attr(&rewritten, "src", &src, &script);
            if let Some(css) = &output.css_bundle {
                let url = dist_url(css)?;
                let link = format!("<link rel=\"stylesheet\" href=\"{url}\">\n");
                let end = rewritten.find("</head>").unwrap_or(0);
                rewritten.insert_str(end, &link);
                css_urls.push(url);
            }
            if server_bundle.is_some() {
                manifest.insert(
                    page.file_name().unwrap().to_string_lossy().into_owned(),
                    ClientAssets {
                        script,
                        css: css_urls,
                    },
                );
            }
        }
        let dest = dist_dir.join(page.file_name().unwrap());
        fs::write(&dest, rewritten)
            .map_err(|e| format!("failed to write {}: {e}", dest.display()))?;
    }

    if let Some(bundle) = server_bundle {
        let server_dir = dist_dir.join("server");
        let manifest = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| format!("failed to serialize SSR client manifest: {e}"))?;
        fs::create_dir_all(&server_dir)
            .and_then(|()| fs::write(server_dir.join("entry.mjs"), bundle))
            .and_then(|()| fs::write(dist_dir.join("ssr-manifest.json"), manifest))
            .map_err(|e| format!("failed to write SSR output: {e}"))?;
    }

    let public_dir = root.join("public");
    if public_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&public_dir) {
            for entry in entries.flatten() {
                let src_path = entry.path();
                if src_path.is_file() {
                    let dest_path = dist_dir.join(entry.file_name());
                    if !dest_path.exists() {
                        let _ = fs::copy(&src_path, &dest_path);
                    }
                }
            }
        }
    }

    print_build_summary(root, &dist_dir, start)
}

pub(crate) fn print_build_summary(root: &Path, dist: &Path, start: Instant) -> Result<(), String> {
    let mut sizes = Vec::new();
    collect_sizes(dist, &mut sizes).map_err(|e| format!("failed to measure output files: {e}"))?;
    sizes.sort_unstable();
    let elapsed = start.elapsed().as_millis();
    println!();
    crate::log::ok(&format!("✓ built in {}ms into dist/", elapsed));
    println!();
    println!("  {:<36} {:>12} {:>12}", "Output", "Raw", "Gzip");
    for (path, raw, gzip) in sizes {
        let name = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        println!(
            "  {}{name:<36}{} {:>8.2} KiB {:>8.2} KiB",
            crate::log::GRAY,
            crate::log::RESET,
            raw as f64 / 1024.0,
            gzip as f64 / 1024.0,
        );
    }
    println!();

    Ok(())
}

fn rewrite_attr(html: &str, attr: &str, original: &str, replacement: &str) -> String {
    html.replace(
        &format!("{attr}=\"{original}\""),
        &format!("{attr}=\"{replacement}\""),
    )
    .replace(
        &format!("{attr}='{original}'"),
        &format!("{attr}='{replacement}'"),
    )
}

fn collect_sizes(dir: &Path, sizes: &mut Vec<(PathBuf, u64, u64)>) -> io::Result<()> {
    for item in fs::read_dir(dir)? {
        let item = item?;
        let kind = item.file_type()?;
        if kind.is_dir() {
            collect_sizes(&item.path(), sizes)?;
        } else if kind.is_file() {
            let file = fs::File::open(item.path())?;
            let raw = file.metadata()?.len();
            let mut encoder = GzEncoder::new(file, Compression::default());
            let gzip = io::copy(&mut encoder, &mut io::sink())?;
            sizes.push((item.path(), raw, gzip));
        }
    }
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
        assert!(run_build(dir.path(), None)
            .unwrap_err()
            .contains("index.html not found"));

        dir.write("index.html", "<html><body></body></html>");
        assert!(run_build(dir.path(), None)
            .unwrap_err()
            .contains("<script type=\"module\""));

        dir.write("index.html", r#"<script type="module" src="/missing.js"></script>"#);
        assert!(run_build(dir.path(), None)
            .unwrap_err()
            .contains("entry script file not found"));
    }
}
