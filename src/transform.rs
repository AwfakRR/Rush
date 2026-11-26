use oxc::{
    allocator::Allocator,
    codegen::Codegen,
    parser::Parser,
    semantic::SemanticBuilder,
    span::SourceType,
    transformer::{TransformOptions, Transformer},
};
use std::path::Path;

/// Transpile TS/TSX/JSX → JS. Returns (code, optional_sourcemap_json).
/// ponytail: one function, no config struct, defaults handle 99% of cases.
/// Custom JSX pragma or tsconfig paths? Add TransformOptions param when needed.
pub fn transpile(filename: &str, source: &str) -> Result<(String, Option<String>), String> {
    let path = Path::new(filename);
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path)
        .map_err(|e| format!("Unknown file type {filename}: {e}"))?;

    // 1. Parse source code into AST
    let parser_ret = Parser::new(&allocator, source, source_type).parse();
    // ponytail: oxc parser is error-recovering. Only fail if AST body is completely empty with errors.
    let mut program = parser_ret.program;
    if program.body.is_empty() && !parser_ret.diagnostics.is_empty() && !source.trim().is_empty() {
        return Err(format!("Parse error in {filename}: {:?}", parser_ret.diagnostics));
    }

    // 2. Semantic analysis (populates scopes and bindings for transformer)
    let semantic_ret = SemanticBuilder::new().build(&program);
    let scoping = semantic_ret.semantic.into_scoping();

    // 3. Transform (TypeScript type stripping + JSX automatic lowering + React Fast Refresh)
    let mut transform_options = TransformOptions::default();
    transform_options.jsx.refresh = Some(Default::default());
    let _ = Transformer::new(&allocator, path, &transform_options)
        .build_with_scoping(scoping, &mut program);

    // 4. Codegen into JavaScript string with source map
    let codegen_options = oxc::codegen::CodegenOptions {
        source_map_path: Some(path.to_path_buf()),
        ..oxc::codegen::CodegenOptions::default()
    };
    let codegen_ret = Codegen::new()
        .with_options(codegen_options)
        .build(&program);

    Ok((codegen_ret.code, codegen_ret.map.map(|sm| sm.to_json_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strips_types() {
        let (code, _) = transpile("test.ts", "const x: number = 42;").unwrap();
        assert!(code.contains("const x = 42"));
        assert!(!code.contains(": number"));
    }

    #[test]
    fn test_transforms_jsx() {
        let (code, _) = transpile("test.jsx", "const el = <div>Hello</div>;").unwrap();
        assert!(!code.contains("<div>"));
    }

    #[test]
    fn test_transforms_tsx() {
        let (code, _) = transpile("test.tsx", "const App = (): string => <h1>Hello</h1>;").unwrap();
        assert!(!code.contains(": string"));
        assert!(!code.contains("<h1>"));
    }

    #[test]
    fn test_transforms_jsx_refresh() {
        let mut opts = TransformOptions::default();
        opts.jsx.refresh = Some(Default::default());
        // Verify transpile handles JSX refresh
        let (code, _) = transpile("Component.tsx", "export function App() { return <h1>Hello</h1>; }").unwrap();
        println!("Transpiled code:\n{code}");
    }

    #[test]
    fn test_jsx_refresh_registers_components() {
        let (code, _) = transpile("App.jsx", "export function App() { return <h1>Hello</h1>; }").unwrap();
        assert!(code.contains("$RefreshReg$"), "{code}");
        assert!(code.contains("react/jsx-runtime"), "{code}");
    }

    #[test]
    fn test_returns_source_map() {
        let (_, map) = transpile("a.ts", "export const a: number = 1;").unwrap();
        let map: serde_json::Value = serde_json::from_str(&map.expect("source map")).unwrap();
        assert_eq!(map["version"], 3);
        assert!(!map["mappings"].as_str().unwrap().is_empty());
    }

    #[test]
    fn test_preserves_imports_and_strips_type_only() {
        let src = "import type { T } from './types';\nimport { f } from './f';\nexport const x: T = f();";
        let (code, _) = transpile("a.ts", src).unwrap();
        assert!(!code.contains("./types"), "{code}");
        assert!(code.contains("./f"), "{code}");
    }

    #[test]
    fn test_unknown_extension_errors() {
        assert!(transpile("styles.css", "body {}").is_err());
    }

    #[test]
    fn test_fatal_parse_error_errors() {
        assert!(transpile("broken.ts", "const = ;").is_err());
    }

    #[test]
    fn test_empty_source_ok() {
        assert!(transpile("empty.ts", "").is_ok());
    }
}
