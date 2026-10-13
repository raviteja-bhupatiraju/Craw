//! Signatures and hover/completion documentation for every name in
//! [`crate::transpiler::BUILTINS`], plus the built-in type names resolved
//! in [`super::resolve`].

/// One builtin's documentation: its display name, a one-line signature,
/// and a short Markdown description.
#[derive(Debug, Clone, Copy)]
pub struct BuiltinDoc {
    pub name: &'static str,
    pub signature: &'static str,
    pub markdown: &'static str,
}

/// Looks up a builtin by name (case-sensitive, matching
/// [`crate::transpiler::BUILTINS`] exactly).
pub fn builtin_doc(name: &str) -> Option<&'static BuiltinDoc> {
    BUILTIN_DOCS.iter().find(|d| d.name == name)
}

/// One built-in *instance* method — dispatched through
/// [`crate::runtime::get_builtin_method`] for `receiver.name(...)` syntax
/// on a string/list/dict/set/int/float, as opposed to a free function
/// called as `name(...)`. `types` names every built-in value kind
/// (matching [`BUILTIN_TYPE_DOCS`]'s names, plus `Frozenset`/`Multiset`)
/// that actually dispatches this method — used to narrow member
/// completion to just the methods that apply once a receiver's type is
/// inferred, and to fall back to every entry when it isn't.
#[derive(Debug, Clone, Copy)]
pub struct BuiltinMethod {
    pub doc: BuiltinDoc,
    pub types: &'static [&'static str],
}

/// Looks up a built-in instance method by name. Distinct table from
/// [`BUILTIN_DOCS`]: some names (`len`, `abs`) exist in both, with
/// different call shapes.
pub fn builtin_method_doc(name: &str) -> Option<&'static BuiltinDoc> {
    BUILTIN_METHODS
        .iter()
        .find(|m| m.doc.name == name)
        .map(|m| &m.doc)
}

/// Every built-in instance method, for member completion on a receiver
/// whose concrete type isn't known statically (dynamic typing means this
/// is a best-effort list, same spirit as offering every field known
/// anywhere in the file) — or, when a type *is* known, filtered by
/// `types` to just the ones that actually apply.
pub fn builtin_methods() -> &'static [BuiltinMethod] {
    BUILTIN_METHODS
}

/// A short description of a built-in type name.
pub fn builtin_type_doc(name: &str) -> Option<&'static str> {
    BUILTIN_TYPE_DOCS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, d)| *d)
}

pub const BUILTIN_TYPE_DOCS: &[(&str, &str)] = &[
    ("Int", "64-bit signed integer."),
    ("Float", "64-bit floating-point number."),
    ("String", "UTF-8 string."),
    ("Bool", "`True` or `False`."),
    ("List", "A mutable, ordered collection: `[1, 2, 3]`."),
    ("Dict", "A hash map: `{\"key\": value}`."),
    (
        "Set",
        "An unordered collection of unique elements: `Set(1, 2, 3)`.",
    ),
    (
        "Tuple",
        "An immutable, fixed-size ordered collection: `(1, 2, 3)`.",
    ),
    ("Dynamic", "No type constraint — accepts any value."),
    ("None", "The absence of a value."),
];

macro_rules! doc {
    ($name:literal, $sig:literal, $md:literal) => {
        BuiltinDoc {
            name: $name,
            signature: $sig,
            markdown: $md,
        }
    };
}

pub const BUILTIN_DOCS: &[BuiltinDoc] = &[
    doc!("abs", "abs(x)", "Absolute value of a number."),
    doc!(
        "Ok",
        "Ok(value)",
        "Constructs a successful `Expected` result."
    ),
    doc!(
        "Err",
        "Err(error)",
        "Constructs a failed `Expected` result."
    ),
    doc!(
        "ident",
        "ident(x)",
        "The identity function: returns its argument unchanged."
    ),
    doc!(
        "const",
        "const(x)",
        "Returns a function that always returns `x`, ignoring its own arguments."
    ),
    doc!(
        "range",
        "range(stop)",
        "A list of integers from `0` up to (excluding) `stop`."
    ),
    doc!(
        "fmap",
        "fmap(f, xs)",
        "Applies `f` to every element of `xs`, returning the results as a new collection."
    ),
    doc!(
        "map",
        "map(f, xs)",
        "Applies `f` to every element of `xs`, returning the results as a new collection."
    ),
    doc!(
        "filter",
        "filter(pred, xs)",
        "Keeps only the elements of `xs` for which `pred` returns a truthy value."
    ),
    doc!(
        "orderby",
        "orderby(xs, key, ascending=True)",
        "Sorts `xs` by the value `key` returns for each element."
    ),
    doc!(
        "reduce",
        "reduce(f, xs)",
        "Combines the elements of `xs` from left to right using `f(accumulator, item)`, starting from the first element."
    ),
    doc!(
        "flatten",
        "flatten(xs)",
        "Flattens one level of nested collections into a single list."
    ),
    doc!(
        "groupsof",
        "groupsof(n, xs)",
        "Splits `xs` into consecutive, non-overlapping chunks of size `n`."
    ),
    doc!(
        "windowsof",
        "windowsof(n, xs)",
        "Every contiguous sliding window of size `n` over `xs`."
    ),
    doc!(
        "enumerate",
        "enumerate(xs)",
        "Pairs each element of `xs` with its index, starting from `0`."
    ),
    doc!(
        "list",
        "list(xs)",
        "Converts an iterable (range, generator, ...) into a `List`."
    ),
    doc!("count", "count(xs)", "The number of elements in `xs`."),
    doc!("len", "len(xs)", "The number of elements in `xs`."),
    doc!(
        "str",
        "str(x)",
        "Converts `x` to its string representation."
    ),
    doc!("any", "any(xs)", "`True` if any element of `xs` is truthy."),
    doc!(
        "all",
        "all(xs)",
        "`True` if every element of `xs` is truthy."
    ),
    doc!("sum", "sum(xs)", "The sum of the elements of `xs`."),
    doc!(
        "product",
        "product(xs)",
        "The product of the elements of `xs`."
    ),
    doc!(
        "thread_map",
        "thread_map(f, xs)",
        "Like `map`, but applies `f` to the elements of `xs` in parallel across OS threads."
    ),
    doc!(
        "cartesian_product",
        "cartesian_product(xs, ys)",
        "Every pair `(x, y)` with `x` from `xs` and `y` from `ys`."
    ),
    doc!(
        "tee",
        "tee(f, x)",
        "Calls `f(x)` for its side effect, then returns `x` unchanged."
    ),
    doc!(
        "safe_call",
        "safe_call(f, *args)",
        "Calls `f(*args)`, catching a panic and returning it as `Err(message)`; returns `Ok(result)` on success."
    ),
    doc!(
        "and_then",
        "and_then(f, expected)",
        "If `expected` is `Ok(v)`, calls `f(v)`; propagates `Err` unchanged otherwise."
    ),
    doc!(
        "next",
        "next(iter)",
        "Advances a generator or lazy list and returns its next value."
    ),
    doc!("not", "not(x)", "Logical negation of `x`'s truthiness."),
    doc!(
        "print",
        "print(*values)",
        "Prints `values` separated by spaces, followed by a newline."
    ),
    doc!(
        "println",
        "println(*values)",
        "Prints `values` separated by spaces, followed by a newline."
    ),
    doc!("add", "add(a, b)", "`a + b`."),
    doc!("sub", "sub(a, b)", "`a - b`."),
    doc!("mul", "mul(a, b)", "`a * b`."),
    doc!("div", "div(a, b)", "`a / b`."),
    doc!("÷", "÷(a, b)", "Integer division of `a` by `b`."),
    doc!("mod", "mod(a, b)", "`a % b`."),
    doc!("eq", "eq(a, b)", "`a == b`."),
    doc!("ne", "ne(a, b)", "`a != b`."),
    doc!("lt", "lt(a, b)", "`a < b`."),
    doc!("le", "le(a, b)", "`a <= b`."),
    doc!("gt", "gt(a, b)", "`a > b`."),
    doc!("ge", "ge(a, b)", "`a >= b`."),
    doc!(
        "hcat",
        "hcat(*arrays)",
        "Horizontally concatenates arrays (side by side, as columns)."
    ),
    doc!(
        "vcat",
        "vcat(*arrays)",
        "Vertically concatenates arrays (stacked, as rows)."
    ),
    doc!(
        "hvcat",
        "hvcat(rows)",
        "Builds a 2D array from nested rows, concatenating horizontally within each row and vertically across rows."
    ),
    doc!("sqrt", "sqrt(x)", "Square root of `x`."),
    doc!("pow", "pow(base, exp)", "`base` raised to the power `exp`."),
    doc!(
        "approx",
        "approx(a, b)",
        "`True` if `a` and `b` are equal within a relative tolerance of `1e-9`."
    ),
    doc!(
        "to",
        "to(a, b)",
        "An inclusive range from `a` to `b` (the `a to b` operator's underlying function)."
    ),
    doc!(
        "until",
        "until(a, b)",
        "An exclusive range from `a` up to `b` (the `a until b` operator's underlying function)."
    ),
];

macro_rules! method {
    ($name:literal, $sig:literal, $md:literal, $types:expr) => {
        BuiltinMethod {
            doc: doc!($name, $sig, $md),
            types: $types,
        }
    };
}

const STRING: &[&str] = &["String"];
const LIST: &[&str] = &["List"];
const DICT: &[&str] = &["Dict"];
const LIST_DICT: &[&str] = &["List", "Dict"];
const LIST_SET: &[&str] = &["List", "Set"];
const DICT_MULTISET: &[&str] = &["Dict", "Multiset"];
const SET_MULTISET: &[&str] = &["Set", "Multiset"];
const SET_FROZENSET: &[&str] = &["Set", "Frozenset"];
const INT_FLOAT: &[&str] = &["Int", "Float"];
// Multiset has no `.len()` in runtime.rs — only `add`/`items` — so it's
// deliberately not in this list, unlike the other collection types.
const ALL_COLLECTIONS: &[&str] = &["String", "List", "Dict", "Set", "Frozenset"];

/// Built-in instance methods, mirroring `get_{string,list,dict,set,
/// frozenset,multiset,int,float}_method` in `runtime.rs`. Deduplicated by
/// name: `len`/`pop`/`remove`/`items`/`add`/`union`/`intersection`/`abs`
/// each exist on more than one type, and get one entry here with a
/// `types` tag covering all of them rather than one entry per type.
pub const BUILTIN_METHODS: &[BuiltinMethod] = &[
    method!(
        "len",
        "x.len()",
        "The number of elements (or, for a string, UTF-8 bytes) in `x`.",
        ALL_COLLECTIONS
    ),
    method!("lower", "s.lower()", "Lowercases a string.", STRING),
    method!("upper", "s.upper()", "Uppercases a string.", STRING),
    method!(
        "strip",
        "s.strip()",
        "Removes leading and trailing whitespace from a string.",
        STRING
    ),
    method!(
        "split",
        "s.split(sep)",
        "Splits a string on `sep`, returning a list of the parts.",
        STRING
    ),
    method!(
        "join",
        "s.join(xs)",
        "Joins the elements of `xs` (converted to strings) with `s` between them.",
        STRING
    ),
    method!(
        "replace",
        "s.replace(from, to)",
        "Replaces every occurrence of `from` with `to` in a string.",
        STRING
    ),
    method!(
        "startswith",
        "s.startswith(prefix)",
        "`True` if the string starts with `prefix`.",
        STRING
    ),
    method!(
        "endswith",
        "s.endswith(suffix)",
        "`True` if the string ends with `suffix`.",
        STRING
    ),
    method!(
        "append",
        "xs.append(x)",
        "Adds `x` to the end of a list.",
        LIST
    ),
    method!(
        "extend",
        "xs.extend(other)",
        "Appends every element of `other` to a list.",
        LIST
    ),
    method!(
        "insert",
        "xs.insert(index, x)",
        "Inserts `x` into a list at `index`.",
        LIST
    ),
    method!(
        "clear",
        "xs.clear()",
        "Removes every element from a list.",
        LIST
    ),
    method!(
        "pop",
        "xs.pop(index=-1) / d.pop(key)",
        "Removes and returns a list's last element (or the one at `index`), or a dict's value for `key`.",
        LIST_DICT
    ),
    method!(
        "remove",
        "xs.remove(x) / s.remove(x)",
        "Removes the first occurrence of `x` from a list, or `x` from a set.",
        LIST_SET
    ),
    method!(
        "get",
        "d.get(key, default=None)",
        "The value for `key` in a dict, or `default` if it isn't present.",
        DICT
    ),
    method!(
        "update",
        "d.update(other)",
        "Merges another dict's entries into `d`, overwriting shared keys.",
        DICT
    ),
    method!(
        "items",
        "d.items()",
        "A list of `[key, value]` pairs from a dict or multiset.",
        DICT_MULTISET
    ),
    method!("keys", "d.keys()", "A list of a dict's keys.", DICT),
    method!("values", "d.values()", "A list of a dict's values.", DICT),
    method!(
        "add",
        "s.add(x)",
        "Adds `x` to a set or multiset.",
        SET_MULTISET
    ),
    method!(
        "union",
        "s.union(other)",
        "Every element in `s` or `other` (a set or frozenset).",
        SET_FROZENSET
    ),
    method!(
        "intersection",
        "s.intersection(other)",
        "Every element in both `s` and `other` (a set or frozenset).",
        SET_FROZENSET
    ),
    method!("abs", "x.abs()", "The absolute value of `x`.", INT_FLOAT),
    method!(
        "Float",
        "i.Float()",
        "Converts an `Int` to a `Float`.",
        &["Int"]
    ),
    method!(
        "Int",
        "f.Int()",
        "Truncates a `Float` to an `Int`.",
        &["Float"]
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transpiler_builtin_has_docs() {
        for (name, _) in crate::transpiler::BUILTINS {
            assert!(
                builtin_doc(name).is_some(),
                "missing docs for builtin {name}"
            );
        }
        let d = builtin_doc("map").unwrap();
        assert_eq!(d.signature, "map(f, xs)");
        assert!(d.markdown.contains("Applies"));
    }

    /// Every entry's `types` tag must be *exact*: the method must resolve
    /// through `runtime::get_builtin_method` for every type it claims,
    /// and must NOT resolve for any type it doesn't — otherwise
    /// completion would either miss a real method or offer one that
    /// panics at runtime. This is what keeps type-narrowed member
    /// completion honest as runtime.rs's method tables evolve.
    #[test]
    fn every_builtin_method_type_tag_is_exact() {
        use crate::runtime::get_builtin_method;

        let by_type = sample_values_by_type();
        for m in BUILTIN_METHODS {
            for (type_name, value) in &by_type {
                let dispatches = get_builtin_method(value, m.doc.name).is_some();
                let tagged = m.types.contains(type_name);
                assert_eq!(
                    dispatches, tagged,
                    "{:?}.types says {tagged} for {type_name}, but runtime dispatch says {dispatches}",
                    m.doc.name
                );
            }
        }
    }

    fn sample_values_by_type() -> Vec<(&'static str, crate::runtime::CrawValue)> {
        use crate::runtime::CrawValue;
        use std::cell::RefCell;
        use std::collections::{HashMap, HashSet};
        use std::rc::Rc;

        vec![
            ("Int", CrawValue::Int(0)),
            ("Float", CrawValue::Float(0.0)),
            ("String", CrawValue::String(Rc::new(String::new()))),
            ("List", CrawValue::List(Rc::new(RefCell::new(Vec::new())))),
            (
                "Dict",
                CrawValue::Dict(Rc::new(RefCell::new(HashMap::new()))),
            ),
            ("Set", CrawValue::Set(Rc::new(RefCell::new(HashSet::new())))),
            ("Frozenset", CrawValue::Frozenset(Rc::new(HashSet::new()))),
            (
                "Multiset",
                CrawValue::Multiset(Rc::new(RefCell::new(HashMap::new()))),
            ),
        ]
    }
}
