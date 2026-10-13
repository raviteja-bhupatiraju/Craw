//! End-to-end protocol conformance: drives the real `craw-lsp` binary over
//! stdio JSON-RPC exactly as an editor would, and checks that every
//! advertised capability is actually served (with well-formed results),
//! that notifications flow, and that the custom `craw/*` requests work.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

struct Server {
    child: Child,
    stdin: ChildStdin,
    messages: mpsc::Receiver<Value>,
    next_id: i64,
    /// Notifications received while waiting for responses.
    notifications: Vec<Value>,
}

fn read_message(reader: &mut BufReader<ChildStdout>) -> Option<Value> {
    let mut len = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length: ") {
            len = v.parse().ok()?;
        }
    }
    let mut buf = vec![0; len];
    reader.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

impl Server {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_craw-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn craw-lsp");
        let stdin = child.stdin.take().unwrap();
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Some(m) = read_message(&mut reader) {
                if tx.send(m).is_err() {
                    break;
                }
            }
        });
        Server {
            child,
            stdin,
            messages: rx,
            next_id: 1,
            notifications: Vec::new(),
        }
    }

    fn write(&mut self, msg: &Value) {
        let body = serde_json::to_vec(msg).unwrap();
        write!(self.stdin, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
        self.stdin.write_all(&body).unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        let mut msg = json!({ "jsonrpc": "2.0", "method": method });
        if !params.is_null() {
            msg["params"] = params;
        }
        self.write(&msg);
    }

    fn recv(&mut self) -> Value {
        let m = self
            .messages
            .recv_timeout(Duration::from_secs(20))
            .expect("server did not answer in time");
        // Answer server→client requests (registerCapability, refresh).
        if m.get("method").is_some() && m.get("id").is_some() {
            let id = m["id"].clone();
            self.write(&json!({ "jsonrpc": "2.0", "id": id, "result": null }));
        }
        m
    }

    /// Sends a request and returns the whole response (`result` or `error`).
    fn request_raw(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let mut msg = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        // Parameterless requests (`shutdown`) must omit `params` entirely.
        if !params.is_null() {
            msg["params"] = params;
        }
        self.write(&msg);
        loop {
            let m = self.recv();
            if m.get("id") == Some(&json!(id)) && m.get("method").is_none() {
                return m;
            }
            if m.get("method").is_some() && m.get("id").is_none() {
                self.notifications.push(m);
            }
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let r = self.request_raw(method, params);
        assert!(r.get("error").is_none(), "{method} failed: {r}");
        r["result"].clone()
    }

    /// Waits for the next `publishDiagnostics` for `uri`.
    fn diagnostics(&mut self, uri: &str) -> Vec<Value> {
        if let Some(i) = self.notifications.iter().position(|n| {
            n["method"] == "textDocument/publishDiagnostics" && n["params"]["uri"] == uri
        }) {
            let n = self.notifications.remove(i);
            return n["params"]["diagnostics"].as_array().unwrap().clone();
        }
        loop {
            let m = self.recv();
            if m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri {
                return m["params"]["diagnostics"].as_array().unwrap().clone();
            }
        }
    }

    fn initialize(&mut self) -> Value {
        self.initialize_with_root(Value::Null)
    }

    fn initialize_with_root(&mut self, root: Value) -> Value {
        let r = self.request(
            "initialize",
            json!({
                "processId": null,
                "rootUri": root,
                "capabilities": {
                    "workspace": { "inlayHint": { "refreshSupport": true } },
                    "textDocument": { "semanticTokens": { "requests": { "full": true, "range": true }, "tokenTypes": [], "tokenModifiers": [], "formats": ["relative"] } }
                },
                "initializationOptions": { "diagnostics": { "unresolved": true, "unused": true } }
            }),
        );
        self.notify("initialized", json!({}));
        r
    }

    fn open(&mut self, uri: &str, text: &str) -> Vec<Value> {
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": uri, "languageId": "craw", "version": 1, "text": text } }),
        );
        self.diagnostics(uri)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn pos(uri: &str, line: u32, character: u32) -> Value {
    json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": character } })
}

const URI: &str = "file:///proto/main.craw";
const SRC: &str = "\
# Doubles a number.
def double(n: Int):
    return n * 2

data Point(x, y)
p = Point(1, 2)
total = double(21)
print(total, p.x)
print(totl)
";

#[test]
fn initialize_advertises_every_capability() {
    let mut s = Server::start();
    let r = s.initialize();
    let caps = &r["capabilities"];
    for key in [
        "hoverProvider",
        "definitionProvider",
        "typeDefinitionProvider",
        "referencesProvider",
        "documentHighlightProvider",
        "renameProvider",
        "documentSymbolProvider",
        "workspaceSymbolProvider",
        "completionProvider",
        "signatureHelpProvider",
        "semanticTokensProvider",
        "foldingRangeProvider",
        "selectionRangeProvider",
        "inlayHintProvider",
        "codeActionProvider",
        "documentFormattingProvider",
        "documentRangeFormattingProvider",
        "callHierarchyProvider",
    ] {
        assert!(
            !caps[key].is_null() && caps[key] != json!(false),
            "{key} not advertised: {caps}"
        );
    }
    assert_eq!(caps["textDocumentSync"], json!(2), "incremental sync");
    let legend = &caps["semanticTokensProvider"]["legend"];
    assert!(
        legend["tokenTypes"]
            .as_array()
            .unwrap()
            .contains(&json!("parameter"))
    );
    assert_eq!(r["serverInfo"]["name"], "craw-lsp");
    assert!(r["serverInfo"]["version"].is_string());
}

#[test]
fn every_request_is_served_over_the_wire() {
    let mut s = Server::start();
    s.initialize();
    let diags = s.open(URI, SRC);
    assert!(
        diags
            .iter()
            .any(|d| d["code"] == "E0100" && d["message"].as_str().unwrap().contains("totl")),
        "{diags:?}"
    );

    let hover = s.request("textDocument/hover", pos(URI, 6, 1));
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("total")
    );

    let items = s.request("textDocument/completion", pos(URI, 7, 6));
    assert!(
        items
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["label"] == "double")
    );

    let sig = s.request("textDocument/signatureHelp", pos(URI, 6, 15));
    assert_eq!(sig["signatures"][0]["label"], "double(n)");

    let def = s.request("textDocument/definition", pos(URI, 6, 9));
    assert_eq!(def["range"]["start"]["line"], 1);

    let tdef = s.request("textDocument/typeDefinition", pos(URI, 7, 13));
    assert_eq!(tdef["range"]["start"]["line"], 4);

    let refs = s.request(
        "textDocument/references",
        json!({ "textDocument": { "uri": URI }, "position": { "line": 1, "character": 5 },
                "context": { "includeDeclaration": true } }),
    );
    assert_eq!(refs.as_array().unwrap().len(), 2);

    let hl = s.request("textDocument/documentHighlight", pos(URI, 6, 1));
    assert_eq!(hl.as_array().unwrap().len(), 2);

    let prep = s.request("textDocument/prepareRename", pos(URI, 6, 1));
    assert_eq!(prep["placeholder"], "total");
    let edit = s.request(
        "textDocument/rename",
        json!({ "textDocument": { "uri": URI }, "position": { "line": 6, "character": 1 }, "newName": "sum_" }),
    );
    assert_eq!(edit["changes"][URI].as_array().unwrap().len(), 2);
    let bad = s.request_raw(
        "textDocument/rename",
        json!({ "textDocument": { "uri": URI }, "position": { "line": 6, "character": 1 }, "newName": "1bad" }),
    );
    assert!(
        bad.get("error").is_some(),
        "invalid rename must be an error: {bad}"
    );

    let syms = s.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": URI } }),
    );
    let names: Vec<_> = syms
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].clone())
        .collect();
    assert!(names.contains(&json!("double")) && names.contains(&json!("Point")));

    let ws = s.request("workspace/symbol", json!({ "query": "dou" }));
    assert!(ws.as_array().unwrap().iter().any(|s| s["name"] == "double"));

    let toks = s.request(
        "textDocument/semanticTokens/full",
        json!({ "textDocument": { "uri": URI } }),
    );
    let data = toks["data"].as_array().unwrap();
    assert!(!data.is_empty() && data.len().is_multiple_of(5));
    let range_toks = s.request(
        "textDocument/semanticTokens/range",
        json!({ "textDocument": { "uri": URI },
                "range": { "start": { "line": 6, "character": 0 }, "end": { "line": 7, "character": 0 } } }),
    );
    assert!(range_toks["data"].as_array().unwrap().len() < data.len());

    let folds = s.request(
        "textDocument/foldingRange",
        json!({ "textDocument": { "uri": URI } }),
    );
    assert!(
        folds
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["startLine"] == 1 && f["endLine"] == 2)
    );

    let sel = s.request(
        "textDocument/selectionRange",
        json!({ "textDocument": { "uri": URI }, "positions": [{ "line": 6, "character": 10 }] }),
    );
    assert!(sel[0]["parent"].is_object());

    let hints = s.request(
        "textDocument/inlayHint",
        json!({ "textDocument": { "uri": URI },
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 9, "character": 0 } } }),
    );
    let labels: Vec<_> = hints
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["label"].clone())
        .collect();
    assert!(labels.contains(&json!("n:")), "{labels:?}");
    assert!(labels.contains(&json!(": Int")), "{labels:?}");
    assert!(labels.contains(&json!("-> Int")), "{labels:?}");

    let actions = s.request(
        "textDocument/codeAction",
        json!({ "textDocument": { "uri": URI },
                "range": { "start": { "line": 8, "character": 7 }, "end": { "line": 8, "character": 7 } },
                "context": { "diagnostics": [] } }),
    );
    assert!(
        actions
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["title"] == "Change to `total`")
    );
    let only_refactors = s.request(
        "textDocument/codeAction",
        json!({ "textDocument": { "uri": URI },
                "range": { "start": { "line": 8, "character": 7 }, "end": { "line": 8, "character": 7 } },
                "context": { "diagnostics": [], "only": ["refactor"] } }),
    );
    assert!(
        only_refactors
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["kind"].as_str().unwrap().starts_with("refactor")),
        "{only_refactors}"
    );

    let fmt = s.request(
        "textDocument/formatting",
        json!({ "textDocument": { "uri": URI }, "options": { "tabSize": 4, "insertSpaces": true } }),
    );
    assert!(
        fmt.as_array().unwrap().is_empty(),
        "sample is already formatted: {fmt}"
    );
    let rfmt = s.request(
        "textDocument/rangeFormatting",
        json!({ "textDocument": { "uri": URI },
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 1, "character": 0 } },
                "options": { "tabSize": 4, "insertSpaces": true } }),
    );
    assert!(rfmt.is_array());

    let items = s.request("textDocument/prepareCallHierarchy", pos(URI, 1, 5));
    let item = items[0].clone();
    assert_eq!(item["name"], "double");
    let incoming = s.request("callHierarchy/incomingCalls", json!({ "item": item }));
    assert_eq!(incoming.as_array().unwrap().len(), 1);
    let outgoing = s.request("callHierarchy/outgoingCalls", json!({ "item": item }));
    assert!(outgoing.as_array().unwrap().is_empty());

    let tree = s.request("craw/syntaxTree", json!({ "textDocument": { "uri": URI } }));
    assert!(tree.as_str().unwrap().contains("FunctionDef"));
    let missing = s.request_raw(
        "craw/syntaxTree",
        json!({ "textDocument": { "uri": "file:///nope.craw" } }),
    );
    assert!(missing.get("error").is_some());

    let rust = s.request("craw/transpile", json!({ "textDocument": { "uri": URI } }));
    assert!(
        rust["rust"].as_str().unwrap().contains("craw_main"),
        "{rust}"
    );

    s.request("shutdown", Value::Null);
    s.notify("exit", Value::Null);
}

#[test]
fn transpile_reports_syntax_errors_as_request_errors() {
    let mut s = Server::start();
    s.initialize();
    let uri = "file:///proto/broken.craw";
    s.open(uri, "x = (\n");
    let r = s.request_raw("craw/transpile", json!({ "textDocument": { "uri": uri } }));
    let err = &r["error"];
    assert!(err.is_object(), "{r}");
    assert!(err["message"].as_str().unwrap().contains("1:"), "{err}");
}

#[test]
fn incremental_changes_update_diagnostics_and_answers() {
    let mut s = Server::start();
    s.initialize();
    let uri = "file:///proto/edit.craw";
    let d = s.open(uri, "value = 1\nprint(valu)\n");
    assert!(d.iter().any(|d| d["code"] == "E0100"));
    // Fix the typo: insert `e` at 1:10.
    s.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "range": { "start": { "line": 1, "character": 10 },
                                               "end": { "line": 1, "character": 10 } }, "text": "e" }] }),
    );
    let d = s.diagnostics(uri);
    assert!(!d.iter().any(|d| d["code"] == "E0100"), "{d:?}");
    let def = s.request("textDocument/definition", pos(uri, 1, 7));
    assert_eq!(def["range"]["start"]["line"], 0);
}

#[test]
fn configuration_changes_filter_diagnostics_and_hints() {
    let mut s = Server::start();
    s.initialize();
    let uri = "file:///proto/cfg.craw";
    let d = s.open(uri, "def f(width):\n    return width\nprint(nope, f(2))\n");
    assert!(d.iter().any(|d| d["code"] == "E0100"));
    s.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": { "craw": {
            "diagnostics": { "unresolved": false, "unused": true },
            "inlayHints": { "parameterNames": { "enable": false } }
        } } }),
    );
    let d = s.diagnostics(uri);
    assert!(!d.iter().any(|d| d["code"] == "E0100"), "{d:?}");
    let hints = s.request(
        "textDocument/inlayHint",
        json!({ "textDocument": { "uri": uri },
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 3, "character": 0 } } }),
    );
    assert!(
        !hints
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["label"] == "width:"),
        "{hints}"
    );
}

#[test]
fn closing_a_document_clears_its_state() {
    let mut s = Server::start();
    s.initialize();
    let uri = "untitled:Untitled-1";
    s.open(uri, "x = 1\nprint(x)\n");
    let hover = s.request("textDocument/hover", pos(uri, 1, 6));
    assert!(hover.is_object(), "untitled documents are served: {hover}");
    s.notify(
        "textDocument/didClose",
        json!({ "textDocument": { "uri": uri } }),
    );
    let hover = s.request("textDocument/hover", pos(uri, 1, 6));
    assert!(hover.is_null());
}

#[test]
fn version_flag_prints_the_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_craw-lsp"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("craw-lsp "));
}

#[test]
fn modules_work_across_files_over_the_wire() {
    let dir = tempfile::tempdir().unwrap();
    let root = craw::modules::normalize(dir.path());
    std::fs::write(
        root.join("geometry.craw"),
        "def area(w, h):\n    return w * h\n",
    )
    .unwrap();
    let file = |name: &str| {
        tower_lsp::lsp_types::Url::from_file_path(root.join(name))
            .unwrap()
            .to_string()
    };
    let main = file("main.craw");
    let geo = file("geometry.craw");
    let main_text = "import geometry\nfrom geometry import volume\nprint(geometry.area(1, 2))\n";
    std::fs::write(root.join("main.craw"), main_text).unwrap();

    let mut s = Server::start();
    s.initialize_with_root(json!(
        tower_lsp::lsp_types::Url::from_directory_path(&root)
            .unwrap()
            .to_string()
    ));
    let d = s.open(&main, main_text);
    assert!(d.iter().any(|d| d["code"] == "E0107"), "{d:?}");

    // Cross-file definition.
    let def = s.request("textDocument/definition", pos(&main, 2, 16));
    assert_eq!(def["uri"], geo);
    assert_eq!(def["range"]["start"]["line"], 0);

    // Opening the module with the missing name republishes main's diagnostics.
    let fixed = "def area(w, h):\n    return w * h\ndef volume():\n    return 1\n";
    s.open(&geo, fixed);
    let d = s.diagnostics(&main);
    assert!(!d.iter().any(|d| d["code"] == "E0107"), "{d:?}");

    // Project-wide rename edits both files.
    let edit = s.request(
        "textDocument/rename",
        json!({ "textDocument": { "uri": main }, "position": { "line": 2, "character": 16 }, "newName": "surface" }),
    );
    let changes = edit["changes"].as_object().unwrap();
    assert!(
        changes.contains_key(&main) && changes.contains_key(&geo),
        "{edit}"
    );

    // Show Generated Rust links the module in.
    let rust = s.request("craw/transpile", json!({ "textDocument": { "uri": main } }));
    assert!(
        rust["rust"].as_str().unwrap().contains("geometry__area"),
        "{rust}"
    );
}
