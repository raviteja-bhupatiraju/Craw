//! `callHierarchy/*`: who calls a function, and what a function calls.
//! A "call" is any read of a callable symbol (a direct call, a pipe
//! `x |> f`, or passing `f` as a value), which matches how dynamically
//! -typed code actually wires functions together.

use crate::semantic::{RefKind, Resolution, SemanticModel, Symbol, SymbolId, SymbolKind};
use crate::text::Span;
use std::collections::BTreeMap;
use tower_lsp::lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyItem, CallHierarchyOutgoingCall, Range, Url,
};

fn callable(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Function
            | SymbolKind::Generator
            | SymbolKind::Method
            | SymbolKind::DataClass
            | SymbolKind::Class
    )
}

/// The item for a callable symbol. `data` records the symbol's name
/// offset, which identifies it again when the client sends the item back.
pub fn item(model: &SemanticModel, uri: &Url, sym: &Symbol) -> CallHierarchyItem {
    let li = model.line_index();
    CallHierarchyItem {
        name: sym.name.clone(),
        kind: crate::lsp_analyzer::lsp_symbol_kind(sym.kind),
        tags: None,
        detail: sym
            .signature
            .as_ref()
            .map(|s| crate::lsp_analyzer::render_signature(&sym.name, s)),
        uri: uri.clone(),
        range: li.range(sym.full_span),
        selection_range: li.range(sym.name_span),
        data: Some(serde_json::json!({ "offset": sym.name_span.start })),
    }
}

/// The item for the symbol under `offset`, if it's callable.
pub fn prepare(model: &SemanticModel, uri: &Url, offset: usize) -> Option<CallHierarchyItem> {
    let r = model
        .references()
        .iter()
        .find(|r| r.span.start <= offset && offset <= r.span.end)?;
    let Resolution::Symbol(id) = r.target else {
        return None;
    };
    let sym = model.symbol(id);
    callable(sym.kind).then(|| item(model, uri, sym))
}

/// The symbol a client-returned item refers to.
pub fn resolve_item(model: &SemanticModel, item: &CallHierarchyItem) -> Option<SymbolId> {
    let offset = item
        .data
        .as_ref()
        .and_then(|d| d.get("offset"))
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .or_else(|| model.line_index().offset(item.selection_range.start.into()))?;
    model
        .symbols()
        .iter()
        .find(|s| s.name_span.start == offset && s.name == item.name && callable(s.kind))
        .map(|s| s.id)
}

/// The innermost callable whose definition contains `span`.
fn enclosing_callable(model: &SemanticModel, span: Span) -> Option<&Symbol> {
    model
        .symbols()
        .iter()
        .filter(|s| {
            matches!(
                s.kind,
                SymbolKind::Function | SymbolKind::Generator | SymbolKind::Method
            ) && s.full_span.start <= span.start
                && span.end <= s.full_span.end
        })
        .min_by_key(|s| s.full_span.len())
}

pub fn incoming(
    model: &SemanticModel,
    uri: &Url,
    target: SymbolId,
) -> Vec<CallHierarchyIncomingCall> {
    let li = model.line_index();
    // Grouped by caller; `None` = module-level code.
    let mut groups: BTreeMap<Option<u32>, Vec<Range>> = BTreeMap::new();
    for r in model.references_to(target) {
        if r.kind != RefKind::Read {
            continue;
        }
        let caller = enclosing_callable(model, r.span).map(|s| s.id.0);
        groups.entry(caller).or_default().push(li.range(r.span));
    }
    groups
        .into_iter()
        .map(|(caller, from_ranges)| {
            let from = match caller {
                Some(id) => item(model, uri, model.symbol(SymbolId(id))),
                None => module_item(model, uri),
            };
            CallHierarchyIncomingCall { from, from_ranges }
        })
        .collect()
}

/// A pseudo-item standing for a file's top-level code.
fn module_item(model: &SemanticModel, uri: &Url) -> CallHierarchyItem {
    let li = model.line_index();
    let name = uri
        .path_segments()
        .and_then(|mut s| s.next_back().map(str::to_string))
        .unwrap_or_else(|| "<module>".to_string());
    let whole = li.range(Span::new(0, model.text().len()));
    CallHierarchyItem {
        name,
        kind: tower_lsp::lsp_types::SymbolKind::FILE,
        tags: None,
        detail: Some("module".to_string()),
        uri: uri.clone(),
        range: whole,
        selection_range: li.range(Span::new(0, 0)),
        data: None,
    }
}

pub fn outgoing(
    model: &SemanticModel,
    uri: &Url,
    source: SymbolId,
) -> Vec<CallHierarchyOutgoingCall> {
    let li = model.line_index();
    let sym = model.symbol(source);
    let mut groups: BTreeMap<u32, Vec<Range>> = BTreeMap::new();
    for r in model.references() {
        if r.kind != RefKind::Read
            || r.span.start < sym.full_span.start
            || r.span.end > sym.full_span.end
        {
            continue;
        }
        let Resolution::Symbol(id) = r.target else {
            continue;
        };
        if !callable(model.symbol(id).kind) {
            continue;
        }
        // A call made by a function nested inside this one belongs to it.
        if enclosing_callable(model, r.span).map(|s| s.id) != Some(source) {
            continue;
        }
        groups.entry(id.0).or_default().push(li.range(r.span));
    }
    groups
        .into_iter()
        .map(|(id, from_ranges)| CallHierarchyOutgoingCall {
            to: item(model, uri, model.symbol(SymbolId(id))),
            from_ranges,
        })
        .collect()
}
