//! Indexes every `.craw` file under one or more workspace roots so
//! workspace-wide symbol search works across files, not just the one
//! currently open. An explicitly opened document's in-memory text always
//! wins over whatever is on disk for that same path.

use super::{SemanticModel, SymbolKind};
use std::collections::HashMap;
use std::path::Path;
use tower_lsp::lsp_types::{Range, Url};

/// One symbol found somewhere in the workspace.
#[derive(Debug, Clone)]
pub struct Hit {
    pub name: String,
    pub kind: SymbolKind,
    pub uri: Url,
    pub name_range: Range,
    pub full_range: Range,
}

#[derive(Debug, Clone)]
struct IndexedSymbol {
    name: String,
    kind: SymbolKind,
    name_range: Range,
    full_range: Range,
}

#[derive(Debug, Default)]
struct FileEntry {
    symbols: Vec<IndexedSymbol>,
}

/// Extracts every top-level (module-scope, no container parent) symbol
/// from `text`, the same set [`crate::lsp_analyzer::LspAnalyzer`] shows in
/// a single file's outline.
fn index_text(text: &str) -> FileEntry {
    let model = SemanticModel::build(text);
    let module = model.module_scope();
    let symbols = model
        .symbols()
        .iter()
        .filter(|s| s.parent.is_none() && s.scope == module)
        .map(|s| IndexedSymbol {
            name: s.name.clone(),
            kind: s.kind,
            name_range: model.line_index().range(s.name_span),
            full_range: model.line_index().range(s.full_span),
        })
        .collect();
    FileEntry { symbols }
}

#[derive(Debug, Default)]
pub struct Workspace {
    files: HashMap<Url, FileEntry>,
}

impl Workspace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Walks `root` for every `*.craw` file (skipping `target/`, `.craw/`,
    /// and `node_modules/`) and indexes each one from disk.
    pub fn add_root(&mut self, root: &Path) {
        self.walk_dir(root);
    }

    fn walk_dir(&mut self, dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if path.is_dir() {
                if matches!(name.as_ref(), "target" | ".craw" | "node_modules") {
                    continue;
                }
                self.walk_dir(&path);
            } else if path.extension().is_some_and(|ext| ext == "craw")
                && let Ok(text) = std::fs::read_to_string(&path)
                && let Ok(uri) = Url::from_file_path(&path)
            {
                self.files.insert(uri, index_text(&text));
            }
        }
    }

    /// Indexes `text` as the current content of `uri`, overriding whatever
    /// was indexed for it from disk. Call this from `did_open`/`did_change`
    /// so unsaved edits are reflected in workspace symbol search.
    pub fn update_open(&mut self, uri: Url, text: &str) {
        self.files.insert(uri, index_text(text));
    }

    pub fn remove(&mut self, uri: &Url) {
        self.files.remove(uri);
    }

    /// Every symbol whose name contains `query` (case-insensitive), across
    /// every indexed file.
    pub fn query_symbols(&self, query: &str) -> Vec<Hit> {
        let needle = query.to_lowercase();
        let mut hits = Vec::new();
        for (uri, entry) in &self.files {
            for sym in &entry.symbols {
                if needle.is_empty() || sym.name.to_lowercase().contains(&needle) {
                    hits.push(Hit {
                        name: sym.name.clone(),
                        kind: sym.kind,
                        uri: uri.clone(),
                        name_range: sym.name_range,
                        full_range: sym.full_range,
                    });
                }
            }
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_craw_files_under_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.craw"), "def alpha() = 1\n").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/b.craw"), "data Beta(x)\n").unwrap();
        let mut ws = Workspace::new();
        ws.add_root(dir.path());

        let hits = ws.query_symbols("a");
        let names: Vec<_> = hits.iter().map(|h| h.name.as_str()).collect();
        assert!(names.contains(&"alpha") && names.contains(&"Beta"));
        assert!(hits.iter().all(|h| h.uri.as_str().ends_with(".craw")));
    }

    #[test]
    fn open_document_overrides_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.craw");
        std::fs::write(&path, "def alpha() = 1\n").unwrap();
        let mut ws = Workspace::new();
        ws.add_root(dir.path());
        assert!(!ws.query_symbols("beta").iter().any(|h| h.name == "beta"));

        let uri = Url::from_file_path(&path).unwrap();
        ws.update_open(uri, "def beta() = 2\n");

        let hits = ws.query_symbols("beta");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "beta");
        // The stale on-disk `alpha` definition is gone now that this URI's
        // open-buffer content has replaced it.
        assert!(ws.query_symbols("alpha").is_empty());
    }
}
