//! Runs every example in the tutorial (`tut/tutorial.md`) and checks its
//! output, so the tutorial can't drift from the language.
//!
//! Conventions in the tutorial:
//! - Each ```` ```craw ```` block is a program, run with both the compiler
//!   (`craw run`) and the interpreter (`crawler`) — unless the paragraph
//!   right before it is `*Compiled programs only.*`.
//! - A ```` ```text ```` block introduced by an `Output:` line is the exact
//!   expected stdout of the program before it.
//! - A block starting `# file: path` is one file of a multi-file program;
//!   consecutive such blocks accumulate until the one for `main.craw`,
//!   which is run with all of them.
//!
//! `tut/` isn't tracked by git, so without it this test does nothing.

use std::path::Path;
use std::process::Command;

#[derive(Debug)]
struct Program {
    /// `(relative path, contents)`, `main.craw` last.
    files: Vec<(String, String)>,
    compiled_only: bool,
    expected: Option<String>,
    /// 1-based line of the program's (last) block, for messages.
    line: usize,
}

struct Block {
    lang: String,
    body: String,
    /// The non-empty lines of prose right before the block.
    before: Vec<String>,
    line: usize,
}

fn blocks(md: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut prose: Vec<String> = Vec::new();
    let mut lines = md.lines().enumerate();
    while let Some((i, line)) = lines.next() {
        if let Some(lang) = line.strip_prefix("```") {
            let mut body = String::new();
            for (_, l) in lines.by_ref() {
                if l.starts_with("```") {
                    break;
                }
                body.push_str(l);
                body.push('\n');
            }
            out.push(Block {
                lang: lang.trim().to_string(),
                body,
                before: std::mem::take(&mut prose),
                line: i + 1,
            });
        } else if !line.trim().is_empty() {
            prose.push(line.trim().to_string());
        }
    }
    out
}

fn programs(md: &str) -> Vec<Program> {
    let blocks = blocks(md);
    let mut out: Vec<Program> = Vec::new();
    let mut pending: Vec<(String, String)> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        if b.lang != "craw" {
            continue;
        }
        let compiled_only = b
            .before
            .last()
            .is_some_and(|l| l == "*Compiled programs only.*");
        let (path, body) = match b.body.strip_prefix("# file: ") {
            Some(rest) => {
                let (path, body) = rest.split_once('\n').unwrap_or((rest, ""));
                (path.trim().to_string(), body.to_string())
            }
            None => ("main.craw".to_string(), b.body.clone()),
        };
        pending.push((path.clone(), body));
        if path != "main.craw" {
            continue;
        }
        let expected = blocks
            .get(i + 1)
            .filter(|next| {
                next.lang == "text" && next.before.last().is_some_and(|l| l == "Output:")
            })
            .map(|next| next.body.clone());
        out.push(Program {
            files: std::mem::take(&mut pending),
            compiled_only,
            expected,
            line: b.line,
        });
    }
    assert!(
        pending.is_empty(),
        "file blocks with no main.craw: {pending:?}"
    );
    out
}

fn run(dir: &Path, compiled: bool) -> (bool, String, String) {
    let out = if compiled {
        Command::new(env!("CARGO_BIN_EXE_craw"))
            .args(["run", "main.craw"])
            .current_dir(dir)
            .output()
    } else {
        Command::new(env!("CARGO_BIN_EXE_crawler"))
            .arg("main.craw")
            .current_dir(dir)
            .output()
    }
    .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn every_tutorial_example_runs_and_prints_what_the_tutorial_says() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tut/tutorial.md");
    let Ok(md) = std::fs::read_to_string(&path) else {
        eprintln!("no tut/tutorial.md; skipping");
        return;
    };
    let programs = programs(&md);
    assert!(
        programs.len() >= 20,
        "found only {} programs",
        programs.len()
    );
    let mut failures = Vec::new();
    for p in &programs {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in &p.files {
            let f = dir.path().join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        let backends: &[bool] = if p.compiled_only {
            &[true]
        } else {
            &[true, false]
        };
        for &compiled in backends {
            let name = if compiled { "craw run" } else { "crawler" };
            let (ok, stdout, stderr) = run(dir.path(), compiled);
            if !ok {
                failures.push(format!("line {}: {name} failed:\n{stdout}{stderr}", p.line));
            } else if let Some(want) = &p.expected
                && stdout != *want
            {
                failures.push(format!(
                    "line {}: {name} printed\n{stdout}but the tutorial says\n{want}",
                    p.line
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n---\n"));
}

#[test]
fn the_harness_reads_the_conventions() {
    let md = "Intro\n\n*Compiled programs only.*\n\n```craw\nprint(1)\n```\n\nOutput:\n\n```text\n1\n```\n\n```craw\n# file: lib.craw\nx = 1\n```\n\n```craw\n# file: main.craw\nimport lib\n```\n\n```text\nnot output\n```\n";
    let p = programs(md);
    assert_eq!(p.len(), 2);
    assert!(p[0].compiled_only);
    assert_eq!(p[0].expected.as_deref(), Some("1\n"));
    assert_eq!(p[1].files.len(), 2);
    assert!(!p[1].compiled_only);
    assert_eq!(p[1].expected, None);
}
