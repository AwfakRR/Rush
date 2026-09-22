use std::collections::{HashMap, HashSet};

/// Module graph tracking imports, reverse importer relationships, and self-accepting boundaries.
/// ponytail: flat HashMap, no petgraph dependency. O(N) BFS is microsecond-fast for dev projects.
#[derive(Default, Debug, Clone)]
pub struct ModuleGraph {
    /// module_path -> set of imported module paths
    imports: HashMap<String, HashSet<String>>,
    /// module_path -> set of modules that import it (reverse index)
    importers: HashMap<String, HashSet<String>>,
    /// modules that have registered import.meta.hot.accept()
    accepting: HashSet<String>,
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
    pub fn find_update_boundaries(&self, changed: &str) -> Option<Vec<String>> {
        let changed_key = normalize_path(changed);

        // If the changed module accepts itself, update it directly
        if self.accepting.contains(&changed_key) {
            return Some(vec![changed_key]);
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
            Some(boundaries)
        }
    }
}

pub fn normalize_path(path: &str) -> String {
    let p = path.replace('\\', "/");
    let trimmed = p.trim_start_matches('/');
    format!("/{trimmed}")
}

pub fn has_hot_accept(source: &str) -> bool {
    // ponytail: fast string scan covers import.meta.hot.accept without full AST walk
    source.contains("import.meta.hot.accept") || source.contains("hot.accept")
}

pub fn inject_hot_preamble(source: &str, module_path: &str) -> String {
    let norm = normalize_path(module_path);
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
"#
    );

    format!("{preamble}\n{source}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path("src\\App.tsx"), "/src/App.tsx");
        assert_eq!(normalize_path("/main.js"), "/main.js");
        assert_eq!(normalize_path("counter.ts"), "/counter.ts");
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
