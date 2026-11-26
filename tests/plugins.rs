//! Real Rush processes exercising Vite/Rollup-compatible plugins.
mod common;

use common::{rush, Fixture, Server};
use std::{fs, path::Path, process::Command};

fn node_available() -> bool {
    let available = Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !available {
        eprintln!("skipping: Node.js is required");
    }
    available
}

fn inline_fixture(config: &str) -> Fixture {
    let project = Fixture::new();
    project
        .write("index.html", r#"<script type="module" src="/app.ts"></script>"#)
        .write("app.ts", r#"
            import message from 'virtual:message';
            export { default as note } from 'virtual:note';
            const value: string = message;
            document.title = value + ' MARKER';
            import('virtual:lazy').then(({ default: text }) => console.log(text));
        "#)
        .write("note.js", "export default 'FROM_DISK';")
        .write("lazy.js", "export default 'LAZY_FROM_DISK';")
        .write(config, r#"
            export default async ({ command, mode }) => {
              let greeting;
              return {
                plugins: [
                  {
                    name: 'post',
                    enforce: 'post',
                    transform(code, id) {
                      if (id.endsWith('/app.ts')) return { code: code.replace('ORDERED NORMAL', 'ORDERED NORMAL POST'), map: null };
                    },
                  },
                  Promise.resolve([
                    {
                      name: 'common',
                      config() { return { nested: { greeting: mode } }; },
                      configResolved(config) {
                        if (config.command !== command || config.mode !== mode) throw new Error('wrong command or mode');
                        greeting = config.nested.greeting;
                      },
                      buildStart() { greeting += ' STARTED'; },
                      resolveId(id) {
                        if (id === 'virtual:message') return '\0message';
                        if (id === 'virtual:lazy') return '\0lazy';
                      },
                      async load(id) {
                        if (id === '\0message') return `export default ${JSON.stringify(greeting)};`;
                        if (id === '\0lazy') return 'export { default } from "./lazy.js";';
                      },
                      transform: {
                        order: 'pre',
                        handler(code, id) {
                          if (id.endsWith('/app.ts')) return code.replace('MARKER', 'ORDERED');
                        },
                      },
                    },
                  ]),
                  {
                    name: 'normal-order',
                    enforce: 'pre',
                    transform(code, id) {
                      if (id.endsWith('/app.ts')) return code.replace('ORDERED', 'ORDERED NORMAL');
                    },
                  },
                  {
                    name: 'common-note',
                    resolveId(id, importer) {
                      if (id === 'virtual:note') return this.resolve('./note.js', importer);
                    },
                  },
                  {
                    name: 'dev-only',
                    apply: 'serve',
                    configureServer(server) {
                      server.middlewares.use('/plugin', (req, res, next) => {
                        if (req.url !== '/status?type=dev') return next();
                        res.writeHead(201, { 'x-rush-plugin': 'active' });
                        res.end('plugin middleware');
                      });
                    },
                    transform: {
                      order: 'post',
                      handler(code, id) {
                        if (id.endsWith('/app.ts')) return code.replace(' POST', ' POST DEV_ONLY');
                      },
                    },
                  },
                  {
                    name: 'build-only',
                    apply: (_config, env) => env.command === 'build',
                    buildEnd(error) {
                      if (!error) this.emitFile({ type: 'asset', fileName: 'plugin-build.txt', source: 'buildEnd' });
                    },
                    transform: {
                      order: 'post',
                      handler(code, id) {
                        if (id.endsWith('/app.ts')) return code.replace(' POST', ' POST BUILD_ONLY');
                      },
                    },
                  },
                ],
              };
            };
        "#);
    project
}

#[tokio::test]
async fn dev_runs_config_resolution_load_transform_and_middleware() {
    if !node_available() {
        return;
    }
    let project = inline_fixture("vite.config.mjs");
    let server = Server::start(project.path(), &[]);
    let app = server.get_text("/app.ts").await;
    assert!(app.contains("ORDERED NORMAL POST DEV_ONLY"), "{app}");
    assert!(
        !app.contains("BUILD_ONLY") && !app.contains(": string"),
        "{app}"
    );
    let imports: Vec<_> = app
        .split('"')
        .filter(|part| part.starts_with("/@id/"))
        .collect();
    assert_eq!(imports.len(), 3, "{app}");
    assert!(server
        .get_text(imports[0])
        .await
        .contains("development STARTED"));
    assert!(server.get_text(imports[1]).await.contains("FROM_DISK"));
    let lazy = server.get_text(imports[2]).await;
    assert!(lazy.contains(r#"from "/lazy.js""#), "{lazy}");
    assert_eq!(server.get("/@id/9999").await.status(), 404);
    let response = server.get("/plugin/status?type=dev").await;
    assert_eq!(response.status(), 201);
    assert_eq!(response.headers()["x-rush-plugin"], "active");
    assert_eq!(response.text().await.unwrap(), "plugin middleware");
    assert_eq!(server.get("/plugin/ignored.js").await.status(), 404);
}

#[cfg(unix)]
fn project_modules(project: &Fixture) -> bool {
    let modules = Path::new(env!("CARGO_MANIFEST_DIR")).join("example/node_modules");
    if !modules.join("esbuild").is_dir() || !modules.join("@rollup/plugin-virtual").is_dir() {
        eprintln!("skipping: npm ci --prefix example is required");
        return false;
    }
    let local = project.path().join("node_modules");
    fs::create_dir(&local).unwrap();
    for package in fs::read_dir(modules).unwrap().flatten() {
        if !package.file_name().to_string_lossy().starts_with('.') {
            std::os::unix::fs::symlink(package.path(), local.join(package.file_name())).unwrap();
        }
    }
    true
}

#[cfg(unix)]
fn build_js(project: &Fixture) -> String {
    let output = rush()
        .args(["build", "--root"])
        .arg(project.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::read_dir(project.path().join("dist/assets"))
        .unwrap()
        .flatten()
        .filter(|file| file.path().extension().is_some_and(|ext| ext == "js"))
        .map(|file| fs::read_to_string(file.path()).unwrap())
        .collect()
}

#[cfg(unix)]
#[test]
fn build_runs_hooks_and_emits_an_asset() {
    if !node_available() {
        return;
    }
    let project = inline_fixture("rush.config.mjs");
    if !project_modules(&project) {
        return;
    }
    let bundle = build_js(&project);
    for expected in [
        "production STARTED",
        "ORDERED NORMAL POST BUILD_ONLY",
        "LAZY_FROM_DISK",
    ] {
        assert!(bundle.contains(expected), "{bundle}");
    }
    assert!(
        !bundle.contains("DEV_ONLY") && !bundle.contains("virtual:message"),
        "{bundle}"
    );
    assert_eq!(
        fs::read_to_string(project.path().join("dist/plugin-build.txt")).unwrap(),
        "buildEnd"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn published_rollup_plugins_work_in_dev_and_build() {
    if !node_available() {
        return;
    }
    let project = Fixture::new();
    if !project_modules(&project) {
        return;
    }
    project
        .write(
            "rush.config.mjs",
            include_str!("../example/rush.config.mjs"),
        )
        .write(
            "vite.config.mjs",
            "throw new Error('rush config should win');",
        )
        .write(
            "index.html",
            r#"<script type="module" src="/app.js"></script>"#,
        )
        .write(
            "app.js",
            r#"
            import welcome from 'virtual:welcome';
            import demo from './plugin-demo.yaml';
            document.title = [welcome, demo.title, __DEMO_MODE__].join(' ');
        "#,
        )
        .write(
            "plugin-demo.yaml",
            include_str!("../example/plugin-demo.yaml"),
        );
    {
        let server = Server::start(project.path(), &[]);
        let main = server.get_text("/app.js").await;
        assert!(main.contains("development"), "{main}");
        let id = main
            .split('"')
            .find(|part| part.starts_with("/@id/"))
            .unwrap();
        assert!(server
            .get_text(id)
            .await
            .contains("Hello from a virtual module"));
        assert!(server
            .get_text("/plugin-demo.yaml")
            .await
            .contains("Powered by published Rollup plugins"));
    }
    let bundle = build_js(&project);
    for expected in [
        "production",
        "Hello from a virtual module",
        "Powered by published Rollup plugins",
    ] {
        assert!(bundle.contains(expected), "{bundle}");
    }
}
