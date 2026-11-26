use oxc::{
    allocator::{Allocator, CloneIn},
    ast::ast::{
        ExportAllDeclaration, ExportFromDeclaration, ImportDeclaration, StringLiteral,
        TSExternalModuleReference, TSImportType,
    },
    ast_visit::{walk_mut, VisitMut},
    codegen::Codegen,
    diagnostics::Diagnostics,
    isolated_declarations::IsolatedDeclarations,
    parser::Parser,
    span::SourceType,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

const ENTRY_EXTENSIONS: [&str; 6] = ["ts", "tsx", "mts", "js", "jsx", "mjs"];

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Manifest {
    source: Option<PathBuf>,
    dependencies: BTreeMap<String, Value>,
    peer_dependencies: BTreeMap<String, Value>,
    optional_dependencies: BTreeMap<String, Value>,
}

impl Manifest {
    fn read(root: &Path) -> Result<Self, String> {
        let path = root.join("package.json");
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("invalid {}: {e}", path.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("failed to read {}: {e}", path.display())),
        }
    }

    fn externals(&self) -> BTreeSet<&str> {
        self.dependencies
            .keys()
            .chain(self.peer_dependencies.keys())
            .chain(self.optional_dependencies.keys())
            .map(String::as_str)
            .chain(["node:*"])
            .collect()
    }
}

fn resolve_entry(
    root: &Path,
    explicit: Option<&Path>,
    manifest: &Manifest,
) -> Result<PathBuf, String> {
    let entry = if let Some(path) = explicit.or(manifest.source.as_deref()) {
        root.join(path)
    } else {
        ENTRY_EXTENSIONS
            .iter()
            .map(|ext| root.join(format!("src/index.{ext}")))
            .find(|path| path.is_file())
            .ok_or("no library entry found; pass --entry, set package.json source, or create src/index.ts (or a JS/TS variant)")?
    };
    entry
        .canonicalize()
        .ok()
        .filter(|path| path.is_file())
        .ok_or_else(|| format!("library entry file not found: {}", entry.display()))
}

pub(crate) fn run_lib_build(root: &Path, explicit: Option<&Path>) -> Result<(), String> {
    let start = Instant::now();
    crate::log::info(&format!("building library from {}", root.display()));

    let manifest = Manifest::read(root)?;
    let entry = resolve_entry(root, explicit, &manifest)?;
    let stem = entry
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("invalid library entry filename: {}", entry.display()))?;
    let source_type = SourceType::from_path(&entry)
        .map_err(|_| format!("unsupported library entry: {}", entry.display()))?;
    let dist = root.join("dist");
    if entry.starts_with(&dist) || dist.canonicalize().is_ok_and(|dir| entry.starts_with(dir)) {
        return Err("library entry must be outside dist/".into());
    }
    let esbuild = crate::resolve::find_esbuild(root)?;
    let declarations = if source_type.is_typescript() {
        emit_declarations(&entry)?
    } else {
        crate::log::warn("JavaScript library entry: skipping TypeScript declarations");
        Vec::new()
    };

    match fs::remove_dir_all(&dist) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("failed to clean {}: {e}", dist.display())),
    }
    fs::create_dir_all(&dist).map_err(|e| format!("failed to create {}: {e}", dist.display()))?;

    let externals = manifest.externals();
    for (format, extension) in [("esm", "mjs"), ("cjs", "cjs")] {
        let output = Command::new(&esbuild)
            .current_dir(root)
            .arg(&entry)
            .args([
                "--bundle",
                "--sourcemap",
                "--target=es2020",
                "--platform=neutral",
                "--main-fields=module,main",
                &format!("--format={format}"),
                &format!("--outfile=dist/{stem}.{extension}"),
            ])
            .args(externals.iter().map(|name| format!("--external:{name}")))
            .output()
            .map_err(|e| format!("failed to execute esbuild: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "esbuild {format} build failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }

    for (path, declaration) in declarations {
        let output = dist.join(path);
        fs::create_dir_all(output.parent().unwrap())
            .and_then(|()| fs::write(&output, declaration))
            .map_err(|e| format!("failed to write {}: {e}", output.display()))?;
    }
    crate::build::print_build_summary(root, &dist, start)
}

fn resolve_type_import(source: &Path, specifier: &str) -> Option<PathBuf> {
    if !matches!(specifier, "." | "..")
        && !specifier.starts_with("./")
        && !specifier.starts_with("../")
    {
        return None;
    }
    let path = source.parent()?.join(specifier);
    let replacements: &[&str] = match path.extension().and_then(|ext| ext.to_str()) {
        Some("js" | "jsx") => &["ts", "tsx", "d.ts"],
        Some("mjs") => &["mts", "d.mts"],
        Some("cjs") => &["cts", "d.cts"],
        _ => &[],
    };
    let extensions = ["ts", "tsx", "d.ts", "mts", "cts"];
    let path = replacements
        .iter()
        .map(|ext| path.with_extension(ext))
        .chain(std::iter::once(path.clone()))
        .chain(extensions.iter().map(|ext| path.with_added_extension(ext)))
        .chain(
            extensions
                .iter()
                .map(|ext| path.join("index").with_extension(ext)),
        )
        .find(|candidate| {
            SourceType::from_path(candidate).is_ok_and(|kind| kind.is_typescript())
                && candidate.is_file()
        })?;
    path.canonicalize().ok()
}

fn declaration_basename(path: &Path) -> PathBuf {
    let path = path.with_extension("");
    if path.extension().is_some_and(|ext| ext == "d") {
        path.with_extension("")
    } else {
        path
    }
}

fn relative_specifier(from: &Path, to: &Path) -> String {
    let from = from.parent().unwrap();
    let common = from
        .components()
        .zip(to.components())
        .take_while(|(a, b)| a == b)
        .count();
    let prefix = "../".repeat(from.components().count() - common);
    let target = to.components().skip(common).collect::<PathBuf>();
    format!(
        "{}{}",
        if prefix.is_empty() { "./" } else { &prefix },
        target.to_string_lossy().replace('\\', "/")
    )
}

struct DeclarationImports<'a, 'src> {
    allocator: &'a Allocator,
    source: &'src Path,
    extension: &'src str,
    pending: &'src mut Vec<PathBuf>,
}

impl<'a> DeclarationImports<'a, '_> {
    fn rewrite(&mut self, specifier: &mut StringLiteral<'a>) {
        let Some(target) = resolve_type_import(self.source, specifier.value.as_str()) else {
            return;
        };
        let output = declaration_basename(&target).with_added_extension(self.extension);
        let replacement = relative_specifier(self.source, &output);
        specifier.value = self.allocator.alloc_str(&replacement).into();
        specifier.raw = None;
        self.pending.push(target);
    }
}

impl<'a> VisitMut<'a> for DeclarationImports<'a, '_> {
    fn visit_import_declaration(&mut self, node: &mut ImportDeclaration<'a>) {
        self.rewrite(&mut node.source);
    }

    fn visit_export_from_declaration(&mut self, node: &mut ExportFromDeclaration<'a>) {
        self.rewrite(&mut node.source);
    }

    fn visit_export_all_declaration(&mut self, node: &mut ExportAllDeclaration<'a>) {
        self.rewrite(&mut node.source);
    }

    fn visit_ts_import_type(&mut self, node: &mut TSImportType<'a>) {
        self.rewrite(&mut node.source);
        walk_mut::walk_ts_import_type(self, node);
    }

    fn visit_ts_external_module_reference(&mut self, node: &mut TSExternalModuleReference<'a>) {
        self.rewrite(&mut node.expression);
    }
}

fn check_diagnostics(path: &Path, source: &str, diagnostics: Diagnostics) -> Result<(), String> {
    if diagnostics.is_empty() {
        return Ok(());
    }
    Err(diagnostics
        .into_iter()
        .map(|diagnostic| {
            let offset = diagnostic
                .labels
                .first()
                .map_or(0, |label| label.offset() as usize);
            let line = source.as_bytes()[..offset.min(source.len())]
                .iter()
                .filter(|&&byte| byte == b'\n')
                .count()
                + 1;
            format!("{}:{line}: {diagnostic}", path.display())
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn emit_declarations(entry: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let dir = entry.parent().unwrap();
    let mut pending = vec![entry.to_path_buf()];
    let mut visited = HashSet::new();
    let mut outputs = BTreeMap::new();
    while let Some(path) = pending.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }
        let relative = path.strip_prefix(dir).map_err(|_| {
            format!(
                "declaration source {} is outside the entry directory {}",
                path.display(),
                dir.display()
            )
        })?;
        let basename = declaration_basename(relative);
        let source = fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        let source_type = SourceType::from_path(&path).unwrap();
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, &source, source_type).parse();
        check_diagnostics(&path, &source, parsed.diagnostics)?;
        pending.extend(
            parsed
                .module_record
                .requested_modules
                .keys()
                .filter_map(|specifier| resolve_type_import(&path, specifier.as_str())),
        );
        let program = if source_type.is_typescript_definition() {
            parsed.program
        } else {
            let result =
                IsolatedDeclarations::new(&allocator, Default::default()).build(&parsed.program);
            check_diagnostics(&path, &source, result.diagnostics)?;
            result.program
        };
        for (declaration, js) in [("d.ts", "js"), ("d.mts", "mjs"), ("d.cts", "cjs")] {
            let mut program = program.clone_in(&allocator);
            DeclarationImports {
                allocator: &allocator,
                source: &path,
                extension: js,
                pending: &mut pending,
            }
            .visit_program(&mut program);
            let output = basename.with_added_extension(declaration);
            let text = Codegen::new().build(&program).code;
            if outputs.insert(output.clone(), text).is_some() {
                return Err(format!("multiple sources would emit {}", output.display()));
            }
        }
    }
    Ok(outputs.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    #[test]
    fn entry_prefers_explicit_then_source_then_index() {
        let root = TempDir::new();
        fs::create_dir(root.path().join("src")).unwrap();
        for file in ["manual.ts", "declared.ts", "src/index.ts", "src/index.js"] {
            fs::write(root.path().join(file), "").unwrap();
        }
        let manifest = Manifest {
            source: Some(PathBuf::from("declared.ts")),
            ..Manifest::default()
        };
        for (explicit, manifest, expected) in [
            (Some(Path::new("manual.ts")), &manifest, "manual.ts"),
            (None, &manifest, "declared.ts"),
            (None, &Manifest::default(), "src/index.ts"),
        ] {
            assert_eq!(
                resolve_entry(root.path(), explicit, manifest).unwrap(),
                root.path().join(expected).canonicalize().unwrap()
            );
        }
        assert!(
            resolve_entry(root.path(), Some(Path::new("missing.ts")), &manifest)
                .unwrap_err()
                .contains("missing.ts")
        );
        assert!(
            resolve_entry(TempDir::new().path(), None, &Manifest::default())
                .unwrap_err()
                .contains("no library entry found")
        );
    }

    #[test]
    fn manifest_externalizes_runtime_dependencies_and_node_prefix() {
        let root = TempDir::new();
        assert_eq!(
            Manifest::read(root.path()).unwrap().externals(),
            BTreeSet::from(["node:*"])
        );
        fs::write(
            root.path().join("package.json"),
            r#"{
            "dependencies": {"runtime": "1", "both": "1"},
            "peerDependencies": {"@scope/peer": "1", "both": "1"},
            "optionalDependencies": {"optional": "1"},
            "devDependencies": {"dev": "1"}
        }"#,
        )
        .unwrap();
        assert_eq!(
            Manifest::read(root.path()).unwrap().externals(),
            BTreeSet::from(["runtime", "both", "@scope/peer", "optional", "node:*"])
        );
    }

    #[test]
    fn declares_annotated_exports_and_reports_the_source_line_for_errors() {
        let root = TempDir::new();
        let entry = root.path().join("index.ts");
        fs::write(
            &entry,
            "export function id(value: number): number { return value; }",
        )
        .unwrap();
        let declarations: BTreeMap<_, _> = emit_declarations(&entry).unwrap().into_iter().collect();
        assert_eq!(
            declarations[Path::new("index.d.ts")].trim(),
            "export declare function id(value: number): number;"
        );
        fs::write(
            &entry,
            "\nexport function id(value: number) { return value; }",
        )
        .unwrap();
        let error = emit_declarations(&entry).unwrap_err();
        assert!(error.contains("index.ts:2:"), "{error}");
        assert!(error.contains("return type"), "{error}");
    }
}
