//! `textDocument/foldingRange`. Once a server provides folding ranges VS
//! Code stops using indentation folding and the language configuration's
//! region markers, so everything a user could fold before must be produced
//! here: indented blocks, multi-line brackets, comment runs, import runs,
//! and `# region` / `# endregion` pairs.

use crate::lexer::Token;
use crate::semantic::SemanticModel;
use std::collections::HashMap;
use tower_lsp::lsp_types::{FoldingRange, FoldingRangeKind};

pub fn folding_ranges(model: &SemanticModel) -> Vec<FoldingRange> {
    let mut out: Vec<FoldingRange> = Vec::new();
    blocks(model, &mut out);
    brackets(model, &mut out);
    comments_and_regions(model, &mut out);
    imports(model, &mut out);

    // VS Code keeps only one range per start line; keep the widest.
    let mut by_start: HashMap<u32, FoldingRange> = HashMap::new();
    for r in out {
        if r.end_line <= r.start_line {
            continue;
        }
        match by_start.get(&r.start_line) {
            Some(existing) if existing.end_line >= r.end_line => {}
            _ => {
                by_start.insert(r.start_line, r);
            }
        }
    }
    let mut result: Vec<_> = by_start.into_values().collect();
    result.sort_by_key(|r| (r.start_line, r.end_line));
    result
}

fn range(start_line: u32, end_line: u32, kind: Option<FoldingRangeKind>) -> FoldingRange {
    FoldingRange {
        start_line,
        start_character: None,
        end_line,
        end_character: None,
        kind,
        collapsed_text: None,
    }
}

/// The line of the last real (non-marker) token strictly before `idx`.
fn last_real_line_before(model: &SemanticModel, idx: usize) -> Option<u32> {
    let tokens = model.tokens();
    (0..idx)
        .rev()
        .find(|&i| !super::is_marker(&tokens[i]))
        .map(|i| {
            let span = model.spans()[i];
            // A multi-line token (a `rust:` block body) ends on its last line
            // of real content, not the line after its trailing newline.
            let end = if span.end > span.start {
                span.end - 1
            } else {
                span.end
            };
            let mut line = model.line_index().position(end).line;
            while line > 0 && model.line_index().line_text(line).trim().is_empty() {
                line -= 1;
            }
            line
        })
}

/// Each `Indent` .. matching `Dedent` pair folds from the header line (the
/// line of the last real token before the `Indent`) to its last line of
/// content.
fn blocks(model: &SemanticModel, out: &mut Vec<FoldingRange>) {
    let tokens = model.tokens();
    let mut stack: Vec<u32> = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        match t {
            Token::Indent => {
                if let Some(header) = last_real_line_before(model, i) {
                    stack.push(header);
                }
            }
            Token::Dedent | Token::Eof => {
                let closes = if matches!(t, Token::Eof) {
                    stack.len()
                } else {
                    1
                };
                for _ in 0..closes {
                    if let Some(header) = stack.pop()
                        && let Some(end) = last_real_line_before(model, i)
                    {
                        out.push(range(header, end, None));
                    }
                }
            }
            _ => {}
        }
    }
}

/// A bracket pair spanning lines folds up to the line before its closer,
/// so the closing bracket stays visible (the same shape VS Code's own
/// bracket folding uses).
fn brackets(model: &SemanticModel, out: &mut Vec<FoldingRange>) {
    let tokens = model.tokens();
    let li = model.line_index();
    for (i, t) in tokens.iter().enumerate() {
        if !matches!(t, Token::LParen | Token::LBracket | Token::LBrace) {
            continue;
        }
        let Some(close) = super::matching_close(tokens, i) else {
            continue;
        };
        let start = li.position(model.spans()[i].start).line;
        let end = li.position(model.spans()[close].start).line;
        // A two-line bracket has nothing to hide without hiding the closer.
        if end > start + 1 {
            out.push(range(start, end - 1, None));
        }
    }
}

fn comment_line(line: &str) -> Option<&str> {
    line.trim_start().strip_prefix('#')
}

fn is_region_start(body: &str) -> bool {
    let b = body.trim_start();
    b == "region" || b.starts_with("region ")
}

fn is_region_end(body: &str) -> bool {
    let b = body.trim_start();
    b == "endregion" || b.starts_with("endregion ")
}

/// Lines covered by string-like tokens that span lines: a `#` there is
/// string content, not a comment.
fn protected_lines(model: &SemanticModel) -> std::collections::HashSet<u32> {
    let li = model.line_index();
    let mut set = std::collections::HashSet::new();
    for (t, s) in model.tokens().iter().zip(model.spans()) {
        if !matches!(
            t,
            Token::String(_) | Token::FStringRaw(_) | Token::BacktickString(_)
        ) {
            continue;
        }
        let a = li.position(s.start).line;
        let b = li.position(s.end).line;
        for l in a..b {
            set.insert(l);
        }
        // The first line of a single-line string holds real code too;
        // only the continuation lines are pure string content.
        if a < b {
            set.remove(&a);
        }
    }
    set
}

fn comments_and_regions(model: &SemanticModel, out: &mut Vec<FoldingRange>) {
    let li = model.line_index();
    let protected = protected_lines(model);
    let mut run_start: Option<u32> = None;
    let mut regions: Vec<u32> = Vec::new();
    let n = li.line_count() as u32;
    for line in 0..=n {
        let body = if line < n && !protected.contains(&line) {
            comment_line(li.line_text(line))
        } else {
            None
        };
        let plain_comment = body.is_some_and(|b| !is_region_start(b) && !is_region_end(b));
        if let Some(b) = body {
            if is_region_start(b) {
                regions.push(line);
            } else if is_region_end(b)
                && let Some(start) = regions.pop()
            {
                out.push(range(start, line, Some(FoldingRangeKind::Region)));
            }
        }
        if plain_comment {
            run_start.get_or_insert(line);
        } else if let Some(start) = run_start.take()
            && line - 1 > start
        {
            out.push(range(start, line - 1, Some(FoldingRangeKind::Comment)));
        }
    }
}

/// Consecutive top-level `from`/`import`/`use` lines fold as one block.
fn imports(model: &SemanticModel, out: &mut Vec<FoldingRange>) {
    let li = model.line_index();
    let mut run: Option<(u32, u32)> = None;
    for line in 0..li.line_count() as u32 {
        let text = li.line_text(line);
        let is_import = ["from ", "import ", "use "]
            .iter()
            .any(|p| text.starts_with(p));
        let blank = text.trim().is_empty();
        if is_import {
            run = Some(match run {
                Some((s, _)) => (s, line),
                None => (line, line),
            });
        } else if !blank
            && let Some((s, e)) = run.take()
            && e > s
        {
            out.push(range(s, e, Some(FoldingRangeKind::Imports)));
        }
    }
    if let Some((s, e)) = run
        && e > s
    {
        out.push(range(s, e, Some(FoldingRangeKind::Imports)));
    }
}
