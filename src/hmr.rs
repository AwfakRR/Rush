use std::collections::{HashMap, HashSet};

/// Module graph tracking imports, reverse importer relationships, and self-accepting boundaries.
/// ponytail: flat HashMap, no petgraph dependency. O(N) BFS is microsecond-fast for dev projects.
#[derive(Default, Debug, Clone)]
pub struct ModuleGraph {
    /// module_path -> set of imported module paths
    imports: HashMap<String, HashSet<String>>,
    /// module_path -> set of modules that import it (reverse index)
    importers: HashMap<String, HashSet<String>>,
    /// modules with explicit HMR acceptance or React Refresh registrations
    accepting: HashSet<String>,
    versions: HashMap<String, u128>,
}

impl ModuleGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record dependencies for a module.
    pub fn update(&mut self, module: &str, deps: &[String], is_accepting: bool) {
        let mod_key = normalize_path(module);

        // Clean up old reverse edges
        if let Some(old_deps) = self.imports.remove(&mod_key) {
            for old_dep in &old_deps {
                if let Some(parents) = self.importers.get_mut(old_dep) {
                    parents.remove(&mod_key);
                }
            }
        }

        // Add new edges
        let mut dep_set = HashSet::new();
        for dep in deps {
            let dep_key = normalize_path(dep);
            self.importers.entry(dep_key.clone()).or_default().insert(mod_key.clone());
            dep_set.insert(dep_key);
        }
        self.imports.insert(mod_key.clone(), dep_set);

        if is_accepting {
            self.accepting.insert(mod_key);
        } else {
            self.accepting.remove(&mod_key);
        }
    }

    /// Find which module(s) can handle the update without full reload.
    /// ponytail: BFS with max depth 50. If no self-accepting boundary, falls back to full reload.
    fn update_boundaries(&self, changed: &str) -> Option<(Vec<String>, HashSet<String>)> {
        let changed_key = normalize_path(changed);

        // If the changed module accepts itself, update it directly
        if self.accepting.contains(&changed_key) {
            return Some((vec![changed_key.clone()], HashSet::from([changed_key])));
        }

        let mut boundaries = Vec::new();
        let mut queue = vec![changed_key.clone()];
        let mut visited = HashSet::new();

        while let Some(current) = queue.pop() {
            if !visited.insert(current.clone()) {
                continue;
            }
            if visited.len() > 50 {
                return None; // Exceeded depth limit, fall back to full reload
            }

            if let Some(parents) = self.importers.get(&current) {
                if parents.is_empty() {
                    return None;
                }
                for parent in parents {
                    if self.accepting.contains(parent) {
                        boundaries.push(parent.clone());
                    } else {
                        queue.push(parent.clone());
                    }
                }
            } else {
                return None; // No importer found to absorb change
            }
        }

        if boundaries.is_empty() {
            None
        } else {
            boundaries.sort();
            boundaries.dedup();
            visited.extend(boundaries.iter().cloned());
            Some((boundaries, visited))
        }
    }

    #[cfg(test)]
    fn find_update_boundaries(&self, changed: &str) -> Option<Vec<String>> {
        self.update_boundaries(changed)
            .map(|(boundaries, _)| boundaries)
    }

    pub fn invalidate(&mut self, changed: &str, timestamp: u128) -> Option<Vec<String>> {
        let (boundaries, invalidated) = self.update_boundaries(changed)?;
        self.versions
            .extend(invalidated.into_iter().map(|path| (path, timestamp)));
        Some(boundaries)
    }

    pub fn updated_import(&self, specifier: &str, path: &str) -> Option<String> {
        self.versions
            .get(path)
            .map(|version| format!("{specifier}?t={version}"))
    }
}

pub fn local_import_path(module: &str, specifier: &str) -> Option<String> {
    if specifier.starts_with("./") || specifier.starts_with("../") {
        let parent = module.rsplit_once('/').map_or("", |(parent, _)| parent);
        Some(normalize_path(&format!("{parent}/{specifier}")))
    } else if specifier.starts_with('/') && !specifier.starts_with("//") {
        Some(normalize_path(specifier))
    } else {
        None
    }
}

/// Root-relative module key: forward slashes, leading "/", "." and ".." segments collapsed.
pub fn normalize_path(path: &str) -> String {
    let p = path.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}

pub fn has_hot_accept(source: &str) -> bool {
    // ponytail: fast string scan covers import.meta.hot.accept without full AST walk
    source.contains("import.meta.hot.accept") || source.contains("hot.accept")
}

pub fn inject_hot_preamble(source: &str, module_path: &str) -> String {
    let norm = normalize_path(module_path).replace('\'', "\\'");
    // ponytail: client-side HMR context wrapper attached to import.meta.hot
    let preamble = format!(
        r#"// [rush] HMR context
if (!window.__rush_hot) window.__rush_hot = new Map();
let __rush_ctx = window.__rush_hot.get('{norm}') || {{
  data: {{}},
  _acceptCallbacks: [],
  _disposeCallbacks: [],
  accept(cb) {{ if (cb) this._acceptCallbacks.push(cb); }},
  dispose(cb) {{ if (cb) this._disposeCallbacks.push(cb); }},
  invalidate() {{ location.reload(); }},
}};
window.__rush_hot.set('{norm}', __rush_ctx);
import.meta.hot = __rush_ctx;
__rush_ctx._refreshUrl = undefined;
"#
    );

    format!("{preamble}\n{source}")
}

pub fn inject_refresh_preamble(source: &str, automatic_boundary: bool) -> String {
    let boundary = if automatic_boundary {
        "import.meta.hot._refreshUrl = import.meta.url;\n"
    } else {
        ""
    };
    format!(
        r#"import __rush_refresh from "/@react-refresh";
const $RefreshReg$ = (type, id) => __rush_refresh.register(type, JSON.stringify([new URL(import.meta.url).pathname, id]));
const $RefreshSig$ = __rush_refresh.createSignatureFunctionForTransform;
{source}
{boundary}"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path("src\\App.tsx"), "/src/App.tsx");
        assert_eq!(normalize_path("/main.js"), "/main.js");
        assert_eq!(normalize_path("counter.ts"), "/counter.ts");
        assert_eq!(normalize_path("./counter.ts"), "/counter.ts");
        assert_eq!(normalize_path("src/./components/../utils.ts"), "/src/utils.ts");
        assert_eq!(normalize_path("../../escape.ts"), "/escape.ts");
        assert_eq!(normalize_path("src//App.tsx"), "/src/App.tsx");
        assert_eq!(normalize_path(""), "/");
    }

    #[test]
    fn test_has_hot_accept_variants() {
        assert!(has_hot_accept("if (import.meta.hot) { import.meta.hot.accept(); }"));
        assert!(has_hot_accept("const hot = import.meta.hot; hot.accept();"));
        assert!(!has_hot_accept("import.meta.hot.dispose(() => {});"));
    }

    #[test]
    fn test_inject_hot_preamble() {
        let out = inject_hot_preamble("export const a = 1;", "src\\a.ts");
        assert!(out.contains("window.__rush_hot.get('/src/a.ts')"));
        assert!(out.contains("import.meta.hot = __rush_ctx;"));
        assert!(out.ends_with("export const a = 1;"));
    }

    #[test]
    fn test_module_graph_keys_are_normalized() {
        let mut graph = ModuleGraph::new();
        graph.update("src\\App.tsx", &["./src/./utils.ts".to_string()], true);
        assert_eq!(graph.find_update_boundaries("src/utils.ts"), Some(vec!["/src/App.tsx".to_string()]));
    }

    #[test]
    fn test_module_graph_transitive_propagation() {
        // App (accepting) -> Page -> utils
        let mut graph = ModuleGraph::new();
        graph.update("/App.tsx", &["/Page.tsx".to_string()], true);
        graph.update("/Page.tsx", &["/utils.ts".to_string()], false);
        assert_eq!(graph.find_update_boundaries("/utils.ts"), Some(vec!["/App.tsx".to_string()]));
    }

    #[test]
    fn invalidation_versions_dependencies_through_the_boundary() {
        let mut graph = ModuleGraph::new();
        graph.update("/App.tsx", &["/Page.tsx".to_string()], true);
        graph.update("/Page.tsx", &["/utils.ts".to_string()], false);
        assert_eq!(
            graph.invalidate("/utils.ts", 42),
            Some(vec!["/App.tsx".to_string()])
        );
        assert_eq!(
            graph.updated_import("./Page.tsx", "/Page.tsx").as_deref(),
            Some("./Page.tsx?t=42")
        );
        assert_eq!(
            graph.updated_import("./utils.ts", "/utils.ts").as_deref(),
            Some("./utils.ts?t=42")
        );
        assert_eq!(graph.invalidate("/orphan.ts", 43), None);
        assert_eq!(graph.updated_import("./orphan.ts", "/orphan.ts"), None);
    }

    #[test]
    fn test_module_graph_multiple_boundaries_sorted_and_deduped() {
        // Diamond: B and C both accept and import shared; D accepts and imports both B and C.
        let mut graph = ModuleGraph::new();
        graph.update("/c.ts", &["/shared.ts".to_string()], true);
        graph.update("/b.ts", &["/shared.ts".to_string()], true);
        graph.update("/d.ts", &["/b.ts".to_string(), "/c.ts".to_string()], true);
        assert_eq!(
            graph.find_update_boundaries("/shared.ts"),
            Some(vec!["/b.ts".to_string(), "/c.ts".to_string()])
        );
    }

    #[test]
    fn test_module_graph_any_unaccepted_root_forces_reload() {
        // shared is imported by an accepting module and by a non-accepting entry with no importers.
        let mut graph = ModuleGraph::new();
        graph.update("/App.tsx", &["/shared.ts".to_string()], true);
        graph.update("/main.js", &["/shared.ts".to_string()], false);
        assert_eq!(graph.find_update_boundaries("/shared.ts"), None);
    }

    #[test]
    fn test_module_graph_unknown_module_forces_reload() {
        let graph = ModuleGraph::new();
        assert_eq!(graph.find_update_boundaries("/never-served.ts"), None);
    }

    #[test]
    fn test_module_graph_update_drops_stale_edges() {
        let mut graph = ModuleGraph::new();
        graph.update("/App.tsx", &["/old.ts".to_string()], true);
        assert!(graph.find_update_boundaries("/old.ts").is_some());

        // App no longer imports old.ts: changing it must not hot-update App anymore.
        graph.update("/App.tsx", &["/new.ts".to_string()], true);
        assert_eq!(graph.find_update_boundaries("/old.ts"), None);
        assert_eq!(graph.find_update_boundaries("/new.ts"), Some(vec!["/App.tsx".to_string()]));
    }

    #[test]
    fn test_module_graph_accept_can_be_removed() {
        let mut graph = ModuleGraph::new();
        graph.update("/App.tsx", &[], true);
        graph.update("/App.tsx", &[], false);
        assert_eq!(graph.find_update_boundaries("/App.tsx"), None);
    }

    #[test]
    fn test_module_graph_cycle_terminates() {
        let mut graph = ModuleGraph::new();
        graph.update("/a.ts", &["/b.ts".to_string()], false);
        graph.update("/b.ts", &["/a.ts".to_string()], false);
        graph.update("/App.tsx", &["/a.ts".to_string()], true);
        assert_eq!(graph.find_update_boundaries("/b.ts"), Some(vec!["/App.tsx".to_string()]));
    }

    #[test]
    fn test_module_graph_depth_limit_forces_reload() {
        // Chain of 60 non-accepting modules capped by an accepting root: exceeds the BFS limit.
        let mut graph = ModuleGraph::new();
        for i in 0..60 {
            graph.update(&format!("/m{}.ts", i + 1), &[format!("/m{i}.ts")], false);
        }
        graph.update("/App.tsx", &["/m60.ts".to_string()], true);
        assert_eq!(graph.find_update_boundaries("/m0.ts"), None);
        assert_eq!(graph.find_update_boundaries("/m55.ts"), Some(vec!["/App.tsx".to_string()]));
    }

    #[test]
    fn test_has_hot_accept() {
        assert!(has_hot_accept("import.meta.hot.accept((mod) => {});"));
        assert!(!has_hot_accept("console.log('hello');"));
    }

    #[test]
    fn test_module_graph_self_accepting() {
        let mut graph = ModuleGraph::new();
        graph.update("/App.tsx", &[], true);
        assert_eq!(graph.find_update_boundaries("/App.tsx"), Some(vec!["/App.tsx".to_string()]));
    }

    #[test]
    fn test_module_graph_propagates_to_parent_boundary() {
        let mut graph = ModuleGraph::new();
        // App.tsx imports /utils.ts. App.tsx is self-accepting, utils.ts is not.
        graph.update("/App.tsx", &["/utils.ts".to_string()], true);
        graph.update("/utils.ts", &[], false);

        // When utils.ts changes, update boundary should be /App.tsx
        assert_eq!(graph.find_update_boundaries("/utils.ts"), Some(vec!["/App.tsx".to_string()]));
    }

    #[test]
    fn test_module_graph_fallback_to_full_reload() {
        let mut graph = ModuleGraph::new();
        // Neither component is accepting
        graph.update("/main.js", &["/utils.ts".to_string()], false);
        graph.update("/utils.ts", &[], false);

        assert_eq!(graph.find_update_boundaries("/utils.ts"), None);
    }
}
