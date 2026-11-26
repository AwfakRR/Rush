use crate::{alias, env, hmr, resolve, transform, AppState};
use axum::{
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{Arc, Mutex},
};

const CONFIG_FILES: &[&str] = &[
    "rush.config.js",
    "rush.config.mjs",
    "rush.config.ts",
    "vite.config.js",
    "vite.config.mjs",
    "vite.config.ts",
];

pub fn find_config(root: &Path) -> Option<PathBuf> {
    CONFIG_FILES
        .iter()
        .map(|file| root.join(file))
        .find(|path| path.is_file())
}

pub fn is_config(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| CONFIG_FILES.contains(&name))
}

pub struct DevPlugins {
    process: Mutex<Process>,
    ids: Mutex<HashMap<String, String>>,
    pub middleware: bool,
}

struct Process {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Deserialize)]
pub struct Resolved {
    pub id: String,
    #[serde(default)]
    pub external: bool,
}

#[derive(Deserialize)]
pub struct Module {
    pub code: String,
}

#[derive(Deserialize)]
pub struct MiddlewareResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Process {
    fn read<T: DeserializeOwned>(&mut self) -> Result<T, String> {
        let mut line = String::new();
        if self
            .stdout
            .read_line(&mut line)
            .map_err(|error| error.to_string())?
            == 0
        {
            return Err("plugin process exited (see Node.js output above)".into());
        }
        let response: Value = serde_json::from_str(&line)
            .map_err(|error| format!("invalid response from plugin process: {error}"))?;
        if let Some(error) = response["error"].as_str() {
            return Err(error.to_owned());
        }
        serde_json::from_value(response["result"].clone()).map_err(|error| error.to_string())
    }
}

impl DevPlugins {
    pub fn start(root: &Path, port: u16, https: bool) -> Result<Option<Arc<Self>>, String> {
        let Some(config) = find_config(root) else {
            return Ok(None);
        };
        let mut child = Command::new("node")
            .current_dir(root)
            .env("NODE_ENV", "development")
            .args([
                "-e",
                concat!(
                    include_str!("vite_plugins.cjs"),
                    "\n",
                    include_str!("plugin_server.cjs")
                ),
                "--",
            ])
            .arg(config)
            .arg(port.to_string())
            .arg(https.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not start Node.js for plugins: {error}"))?;
        let mut process = Process {
            stdin: child.stdin.take(),
            stdout: BufReader::new(child.stdout.take().unwrap()),
            child,
        };
        let middleware = process.read()?;
        Ok(Some(Arc::new(Self {
            process: Mutex::new(process),
            ids: Mutex::new(HashMap::new()),
            middleware,
        })))
    }

    async fn call<T: DeserializeOwned + Send + 'static>(
        self: &Arc<Self>,
        operation: &'static str,
        args: Value,
    ) -> Result<T, String> {
        let plugins = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut process = plugins.process.lock().unwrap();
            writeln!(
                process.stdin.as_mut().unwrap(),
                "{}",
                json!({ "operation": operation, "args": args })
            )
            .map_err(|error| format!("could not send request to plugin process: {error}"))?;
            process.read()
        })
        .await
        .map_err(|error| error.to_string())?
    }

    async fn resolve(
        self: &Arc<Self>,
        sources: Vec<&str>,
        importer: &str,
    ) -> Result<Vec<Option<Resolved>>, String> {
        self.call("imports", json!([sources, importer])).await
    }

    pub async fn module(
        self: &Arc<Self>,
        id: &str,
        disk_file: Option<&Path>,
    ) -> Result<Option<Module>, String> {
        self.call("module", json!([id, disk_file])).await
    }

    pub async fn middleware(
        self: &Arc<Self>,
        url: &str,
        method: &str,
        headers: HashMap<&str, &str>,
    ) -> Result<Option<MiddlewareResponse>, String> {
        self.call(
            "middleware",
            json!({ "url": url, "method": method, "headers": headers }),
        )
        .await
    }

    pub fn registered_id(&self, url: &str) -> Option<String> {
        self.ids.lock().unwrap().get(url).cloned()
    }

    pub async fn rewrite_imports(
        self: &Arc<Self>,
        code: &str,
        filename: &str,
        importer: &str,
        root: &Path,
        encoded: bool,
    ) -> Result<String, String> {
        let imports = resolve::scan_module_imports(code, filename, true);
        if imports.is_empty() {
            return Ok(code.to_owned());
        }
        let resolved = self
            .resolve(
                imports.iter().map(|(spec, _, _)| spec.as_str()).collect(),
                importer,
            )
            .await?;
        let mut output = code.to_owned();
        for ((spec, start, end), resolved) in imports.into_iter().zip(resolved).rev() {
            let url = if let Some(resolved) = resolved {
                if resolved.external {
                    resolved.id
                } else {
                    let url = module_url(&resolved.id);
                    self.ids.lock().unwrap().insert(url.clone(), resolved.id);
                    url
                }
            } else if crate::resolve::is_bare(&spec) {
                crate::resolve::dep_url("/node_modules/.rush/deps", &spec)
            } else if encoded && spec.starts_with('.') {
                let parent = Path::new(clean_id(importer))
                    .parent()
                    .and_then(|parent| parent.strip_prefix(root).ok())
                    .unwrap_or(Path::new(""));
                crate::hmr::normalize_path(&parent.join(spec).to_string_lossy())
            } else {
                continue;
            };
            output.replace_range(start as usize..end as usize, &json!(url).to_string());
        }
        Ok(output)
    }
}

pub const ID_PREFIX: &str = "/@id/";

fn module_url(id: &str) -> String {
    use std::fmt::Write;
    let mut url = ID_PREFIX.to_owned();
    for byte in id.bytes() {
        write!(url, "{byte:02x}").unwrap();
    }
    url
}

pub fn clean_id(id: &str) -> &str {
    id.split(['?', '#']).next().unwrap_or_default()
}

pub async fn rewrite_imports(
    state: &AppState,
    code: &str,
    filename: &str,
    target: &Path,
) -> Result<String, String> {
    if let Some(plugins) = &state.plugins {
        if !target.starts_with(state.root.join("node_modules")) {
            return plugins
                .rewrite_imports(
                    code,
                    filename,
                    &target.to_string_lossy(),
                    &state.root,
                    false,
                )
                .await;
        }
    }
    Ok(resolve::rewrite_bare_imports(
        code,
        filename,
        "/node_modules/.rush/deps",
    ))
}

pub fn middleware_response(output: MiddlewareResponse) -> Response {
    let status = StatusCode::from_u16(output.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = (status, output.body).into_response();
    for (name, value) in output.headers {
        if let (Ok(name), Ok(value)) = (HeaderName::try_from(name), HeaderValue::try_from(value)) {
            response.headers_mut().insert(name, value);
        }
    }
    response
}

pub fn error_response(error: String) -> Response {
    crate::log::error(&format!("plugin error: {error}"));
    (StatusCode::INTERNAL_SERVER_ERROR, error).into_response()
}

pub async fn serve_module(
    state: &AppState,
    plugins: &Arc<DevPlugins>,
    uri: &Uri,
    headers: &HeaderMap,
    target: &Path,
    extension: &str,
) -> Result<Option<Response>, String> {
    let url = uri.path();
    let encoded = url.starts_with(ID_PREFIX);
    let id = if encoded {
        let Some(id) = plugins.registered_id(url) else {
            return Ok(Some(StatusCode::NOT_FOUND.into_response()));
        };
        id
    } else {
        if extension == "html"
            || target.starts_with(state.root.join("node_modules"))
            || !target.is_file()
        {
            return Ok(None);
        }
        if target.parent() == Some(state.root.as_path()) && is_config(target) {
            return Ok(Some(StatusCode::NOT_FOUND.into_response()));
        }
        let mut id = target.to_string_lossy().into_owned();
        if let Some(query) = uri.query() {
            id.push('?');
            id.push_str(query);
        }
        id
    };
    let file = Path::new(clean_id(&id));
    let extension = file.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    let native_js = matches!(extension, "js" | "mjs" | "ts" | "mts" | "jsx" | "tsx");
    let script_request = headers
        .get("sec-fetch-dest")
        .is_some_and(|value| value == "script");
    let text_module = matches!(
        extension,
        "json" | "yaml" | "yml" | "svg" | "txt" | "md" | "vue" | "svelte"
    );
    if !encoded && !native_js && !script_request && !text_module {
        return Ok(None);
    }
    let disk_file = tokio::fs::canonicalize(file)
        .await
        .ok()
        .filter(|file| file.starts_with(&state.root) && file.is_file());
    if !encoded && disk_file.is_none() {
        return Ok(None);
    }
    let transform_file = if native_js || text_module {
        disk_file.as_deref()
    } else {
        None
    };
    let module = plugins.module(&id, transform_file).await?;
    let code = match (module, &disk_file) {
        (Some(module), _) => module.code,
        (None, Some(file)) if encoded && native_js => tokio::fs::read_to_string(file)
            .await
            .map_err(|error| error.to_string())?,
        (None, _) if encoded => return Ok(Some(StatusCode::NOT_FOUND.into_response())),
        (None, _) => return Ok(None),
    };
    let path = disk_file
        .as_ref()
        .and_then(|file| file.strip_prefix(&state.root).ok())
        .map(|file| hmr::normalize_path(&file.to_string_lossy()));
    let mut transformed = None;
    if matches!(extension, "ts" | "mts" | "tsx" | "jsx") {
        let filename = path.as_deref().unwrap_or_else(|| clean_id(&id));
        match transform::transpile(filename, &code) {
            Ok(output) => transformed = Some(output),
            Err(error) => {
                crate::publish_transpile_error(&state.tx, &state.diagnostics, filename, &error);
                return Ok(Some(
                    (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
                ));
            }
        }
    }
    let explicit = hmr::has_hot_accept(&code);
    let refresh = transformed
        .as_ref()
        .is_some_and(|output| output.refresh_registration);
    let code = transformed
        .as_ref()
        .map_or(code.as_str(), |output| &output.code);
    let alias_code = alias::resolve_aliases(code, "module.js", &state.aliases);
    let mut rewritten = plugins
        .rewrite_imports(&alias_code, "module.js", &id, &state.root, encoded)
        .await?;
    if let Some(path) = &path {
        rewritten = crate::register_module(
            state,
            &rewritten,
            "module.js",
            path,
            !encoded && (explicit || refresh),
        );
    }
    if let Some(output) = transformed {
        if refresh || output.refresh_signature {
            rewritten = hmr::inject_refresh_preamble(&rewritten, !encoded && refresh && !explicit);
        }
        if let (Some(path), Some(map)) = (&path, output.map) {
            let map_path = format!("{path}.map");
            state
                .source_maps
                .write()
                .unwrap()
                .insert(map_path.clone(), map);
            rewritten.push_str(&format!("\n//# sourceMappingURL={map_path}\n"));
        }
    }
    let code = hmr::inject_hot_preamble(&rewritten, url);
    let code = env::replace_env_vars(&code, &state.env_vars);
    Ok(Some(
        ([(header::CONTENT_TYPE, "application/javascript")], code).into_response(),
    ))
}
