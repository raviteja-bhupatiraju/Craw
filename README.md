# Craw

Craw is a high-level language that transpiles to Rust. It's an experiment in
ergonomics - an attempt to bring together productive features for cases where
Rust's zero-cost abstractions and strict type safety are more rigor than
warranted.

At the same time, Craw is source compatible with Rust: you can inline or include
raw Rust source whenever you need it, striking a balance between productivity
and control.

It is influenced by other languages that similarly attempt to extend the
productivity of popular languages: Xtend (Java), Nim \(C\) and Coconut (Python).

## Highlights

- **Transpiles to Rust** --- leverages Rust's ecosystem and performance under
  the hood
- **Small runtime** --- compiled binaries land between Rust and Go in size
- **Interpreter included** --- run Craw directly without a full compile step
- **Rust interop** --- inline or include Rust source when you need the extra
  power
- **VS Code extension** --- basic editor support included in the project

## Status

This is an experimental project. Expect rough edges and sparse development.
The "Language at a glance" section below only shows syntax that actually
works today, verified against the current build.

## Installation

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install), stable channel (edition
  2024, so a recent toolchain — `rustup update` if `cargo build` complains
  about the edition). The repo pins itself to `stable` via
  `rust-toolchain.toml`.
- **`cargo build` can occasionally crash `rustc` itself** (a SIGSEGV inside
  `librustc_driver`/LLVM during codegen, not a normal compile error — look
  for `signal: 11, SIGSEGV` in the output) rather than produce a binary. This
  is an upstream rustc/LLVM instability triggered by this project's
  deeply-generic recursive `chumsky` parser-combinator types; it's already
  mitigated as much as is practical (disabled LTO, `codegen-units = 1`,
  pinned to `stable` — see the git history), but it hasn't been possible to
  eliminate outright, and it isn't nightly-specific: it has been observed on
  the pinned stable toolchain too. It's nondeterministic — if a build fails
  this way, just run `cargo build --release` again; a retry has always
  succeeded so far. If you see this and it persists, it's a compiler/build
  issue, not a bug in your Craw program.
- `rustc` on your `PATH` — the `craw build`/`craw run` commands shell out to it
  to compile the transpiled Rust output

### Build from source

```sh
git clone https://github.com/raviteja-bhupatiraju/Craw.git
cd Craw
cargo build --release
```

The compiled binary is written to `target/release/craw` (`craw.exe` on Windows).

### Install the `craw` CLI

```sh
cargo install --path .
```

This installs `craw` to `~/.cargo/bin` (make sure that directory is on your
`PATH`). Verify with:

```sh
craw --version
```

### Usage

```sh
craw new <name>              # create a new project
craw run <file.craw>         # build and run a Craw file
craw run <file.craw> --release
craw build <file.craw>       # build without running
craw transpile <file.craw>   # transpile to Rust source

crawler <file.craw>          # run a file in the interpreter (no rustc needed)
crawler                      # interactive REPL
```

`craw build`/`craw run` write the transpiled `.rs` and the compiled binary
into a `.craw/` directory next to the source file, keyed by build profile, so
switching between `craw run x.craw` and `craw run --release x.craw` always
rebuilds instead of silently reusing the wrong profile's binary.

Syntax errors are reported as `file:line:col: error: message` by both `craw`
and `crawler`.

### VS Code extension

The extension lives in `editors/vscode/` and bundles the `craw-lsp` language
server (diagnostics, completion, signature help, hover, go to definition,
rename, outline, run/build/transpile commands). Build and install it with:

```sh
scripts/build_extension.sh --install     # or scripts\build_extension.ps1 -Install
```

which builds `craw-lsp`, packages `craw.vsix` in the repo root, and runs
`code --install-extension` on it. See `editors/vscode/README.md` for the
settings and commands it contributes.

### Scripts

Dev scripts live in `scripts/` (each has a `.sh` and a `.ps1` version):

- `build_extension` — builds `craw-lsp` and the VS Code extension from source and packages them into `craw.vsix` (`--install` also installs it).
- `run_samples` — runs `samples/showcase.craw` with `craw run` and reports success/failure.
- `test` — builds the compiler in release mode and runs the feature-test and master-showcase samples to verify everything works end to end.

# Language at a glance

Every example below was run against the current build through both `craw run`
(transpiler) and `crawler` (interpreter); differences between the two are
called out inline where they exist. Anything not shown here — algebraic
effects, sum types, refinement/opaque types, row polymorphism, match types,
`given`/`using`, units of measure, uncertainty arithmetic, symbolic
differentiation, async/await, `comptime`, hygienic macros, and a handful of
syntax forms that parse but don't actually work — is aspirational and is not
documented here.

## 1. Basic types & literals

### Scalars

- `Int` (64-bit), `Float` (64-bit), `String` (UTF-8), `Bool` (`True`/`False`),
  `None`

```python
7 / 2          # 3.5   — `/` is always float division
7 // 2         # 3     — integer division (`÷` is the same thing)
-7 % 3         # -1    — remainder keeps the sign of the dividend, like Rust
2 ** 10        # 1024
str(5)         # "5"   — the only conversion builtin; there is no int()/float()
```

### Collections

- List: `[1, 2, 3]` (mutable)
- Tuple: `(1, 2, 3)` (immutable)
- Dict: `{"key": value}`
- Set: `{1, 2, 3}`, `s{1, 2, 3}` or `Set(1, 2, 3)` — Frozenset: `f{1, 2}` or
  `Frozenset(1, 2)` — Multiset: `m{1, 1, 2}` or `Multiset(1, 1, 2)`

Sets print in hash order (`s{3, 1, 2}`), and the order differs between the two
backends — don't rely on it.

### Arrays

- N-dimensional: `[1, 2; 3, 4]` — semicolons delimit rows, commas delimit
  columns. Index with `M[row, col]` (`M[1, 0]` → `3`); prints as `[1 2; 3 4]`.

### String literals

```python
f"result is {x * 2}"        # interpolation
f"hello {name}"

msg = """
hello
world
"""                          # triple-quoted — content is kept verbatim,
                              # including the leading/trailing newline; no
                              # automatic dedenting is applied

"a" + "b"                    # "ab"
"hi" * 2                     # "hihi"
```

> Format specs inside f-strings are not supported: `f"{x:.2f}"` prints the
> literal text `x:.2f`.
>
> `len()` on a string counts **bytes**, not characters (`len("héllo")` → `6`),
> while indexing and slicing (`s[1]` → `"é"`, `s[::-1]`) work per character.

### Numeric literals

```python
0xFF          # hex        -> 255
0o17          # octal      -> 15
0b1010        # binary     -> 10
1_000_000     # digit-group separators
1e9           # scientific notation -> 1000000000.0
1.5e-3        # -> 0.0015
```

---

## 2. Control flow

### Conditionals

```python
if cond:
    body
else:
    body
```

There is no `elif` (it parses as a call to an undefined macro and the
transpiler aborts) — nest an `if` inside the `else:` block instead. There is
also no `pass`: every block needs at least one real statement.

### Ternary

```python
y = if x > 0 then "pos" else "non-pos"
z = "yes" if x > 0 else "no"          # Python-style form works too
```

### Loops

```python
while cond:
    body

for item in collection:      # lists, tuples, strings (per character), dicts (keys)
    body

for i in 1..4:               # inclusive range literal: 1, 2, 3, 4
    body
for i in range(1, 5):        # exclusive upper bound: 1, 2, 3, 4
    body
range(0, 10, 3)              # [0, 3, 6, 9] — step; range literals have no step form
```

`break` and `continue` work. Augmented assignment (`x += 1`, `-=`, `*=`, and
`+=` on strings) works, and `;` separates statements on one line
(`a = 0; b = 1`). Iterating a dict yields its keys in unspecified order.

### Pattern matching

`match` is a statement, not an expression (`r = match x:` is a parse error),
and each `case` needs its own indented body on the following line(s) —
`case pattern: statement` all on one line is a parse error.

```python
match value:
    case Point(0, y):
        "on Y axis"           # data-class constructor pattern
    case (1, x, _):
        "tuple starts with 1"
    case [a, b]:
        "two-element list"
    case 0:
        "literal"             # ints, strings and None all match literally
    case None:
        "nothing"
    case n if n > 0:
        "positive number"     # guard clause
    case _:
        "default"
```

### View patterns

A view pattern runs a function on the matched value first, then matches the
*result* against a sub-pattern — the syntax is `(function) -> pattern`, not
`function(pattern)`:

```python
match n:
    case (abs) -> m if m < 1e-9:
        "near zero"
    case _:
        "not zero"
```

### String split patterns

A `"prefix" + name` pattern matches a string that starts with the prefix and
binds the rest:

```python
match s:
    case "id:" + rest:
        rest                  # "id:42" -> "42"
    case _:
        "none"
```

### Destructuring

Tuple and list destructuring work, in plain assignments, as `for` targets,
and in parameter lists:

```python
(a, b, c) = some_tuple
(a, b) = (b, a)                  # swap
[px, py] = [10.5, 20.5]
[head, *tail] = [1, 2, 3, 4, 5]  # head = 1, tail = [2, 3, 4, 5]

for (x, y) in points:
    plot(x, y)

def dist((x, y)) = sqrt(x * x + y * y)
```

`dict.items()` yields two-element *lists*, so destructure them with square
brackets: `for [k, v] in d.items():` works, `for (k, v) in d.items():` fails
with a match error.

> Struct/record-field destructuring (`{name, age} = person`, and
> `for {name, age} in records:`) is not implemented in either backend — it's
> a parse error.

### Comprehensions

```python
[n * n for n in range(5)]              # [0, 1, 4, 9, 16]
[n for n in range(10) if n % 2 == 0]   # [0, 2, 4, 6, 8]
[c for c in "xy"]                      # ["x", "y"]
```

Only **list** comprehensions exist. Dict and set comprehensions
(`{k: v for ...}`, `{x for ...}`) are parse errors. A parenthesised lazy
comprehension `(n * n for n in range(3))` works under `craw run` (drive it
with `next()`) but the interpreter rejects it.

---

## 3. Functions & closures

### Defining functions

The function keyword is `def`, **not** `fn` — `fn` is reserved for inline
Rust (see [Rust interop](#9-rust-interop)), and anything written after a bare
`fn` is spliced verbatim into the generated Rust source rather than parsed as
Craw. A `def` with a single-expression body uses `= expr`; a multi-line body
is an indented block and needs an explicit `return` (there's no implicit
last-expression return).

```python
def add(a, b) = a + b                # one-liner
def add(a, b) -> Int = a + b         # optional return-type annotation

def greet(name):
    return "hello, " + name
```

Function parameters are dynamically typed — a per-parameter annotation like
`def f(x: Int)` is not valid syntax; only the function-level `-> ReturnType`
is. Recursion and nested `def`s (closures capturing the enclosing scope) work
in both backends:

```python
def adder(n):
    return x => x + n
add5 = adder(5)
add5(1)                              # 6
```

### Lambdas

```python
add1 = x => x + 1        # canonical short form
mul = (a, b) => a * b    # multiple parameters
sq = lambda x: x * x     # single-expression only — no multi-line lambda body
```

### Implicit lambdas & partial application

```python
inc = (_ + 1)                 # implicit lambda — shorthand for x => x + 1
add_ten = add$(10)            # partial application: waits for the remaining args
partial_add = add$(_, 1)      # partial with a hole: x => add(x, 1)
```

> An implicit lambda only supports a single placeholder use. **Multiple
> `_`s in the same expression are not distinct positional holes** — every
> `_` is silently aliased to the same (first) argument, and any further
> arguments are discarded without an error. `(_ + _)` called with `(3, 4)`
> quietly returns `6` (`3 + 3`), not `7`. For a real two-argument function,
> use a named lambda instead: `(a, b) => a + b` or `lambda a, b: a + b`.
>
> **Always give an implicit lambda its own parentheses when it's an argument.**
> `xs |> filter((_ % 2 == 0))` works; `print(xs |> filter(_ % 2 == 0))` turns
> the *entire statement* into a lambda that is never called, so it prints
> nothing and reports no error.

### Piping

```python
data |> filter(over_18) |> map(get_name)
maybe |?> transform            # None-aware pipe: skips if LHS is None
[1, 2] |*> add                 # splat pipe: unpacks a *list* into the arguments
sq <| 4                        # backward pipe
1 |> add$(_, 10)               # pipe into a partial -> 11
```

`|*>` needs a list on the left; a tuple fails with `star pipe expects a list`.

### Composition

```python
h = double ∘ inc               # h(x) == double(inc(x))
g = inc >> double              # forward: g(x) == double(inc(x))
k = double << inc              # backward: same as ∘
```

### Broadcasting

```python
[1, 2, 3] .+ [10, 20, 30]      # [11, 22, 33]
[1, 2, 3] .+ 10                # [11, 12, 13] — scalars broadcast
a .- b
a .* b
a ./ b
double.(nums)                  # broadcast call: apply a function element-wise
```

### Generators

```python
gen naturals():
    n = 0
    while True:
        yield n
        n = n + 1

g = naturals()
print([next(g) for _ in range(5)])   # [0, 1, 2, 3, 4]
```

Drive a generator with `next()`. There's no "take"-style builtin, and
`for x in naturals():` does **not** work — the transpiled program panics with
`expected an iterable` and the interpreter silently runs zero iterations.

### Named & optional arguments, varargs

```python
def solve(a, b, tol=1e-9):
    return a + b + tol

solve(1.0, 2.0)                 # tol uses its default -> 3.000000001
solve(1.0, 2.0, 1e-6)           # positional override  -> 3.000001
```

```python
def sum3(*xs):
    return xs |> reduce((a, b) => a + b)

nums = [1, 2, 3]
sum3(*nums)                     # splat a collection into varargs -> 6
```

> Keyword arguments are **not matched by name**. `f(name=value)` at a call
> site parses, but the `name=` label is discarded and the value is bound
> positionally, in call order — `f(b=2, a=1)` binds `a=2, b=1` (first
> argument to first parameter, regardless of its label), not `a=1, b=2` as
> the label would suggest. Only rely on keyword syntax when arguments are
> already in declaration order.

### where clause

```python
def f(x, y) = x + y
result = f(a, b) where:
    a = 10                # may reference outer scope
    b = a * 2              # sequential evaluation: b may reference a
```

### Builtins and methods

Builtin functions that exist in both backends:

```python
print  len  str  sqrt  abs  sum  range  next  map  filter  reduce  enumerate
```

`reduce` takes either order: `reduce(f, xs)` or `xs |> reduce(f)`.
`enumerate(xs)` returns a list of `[index, item]` pairs.

**Not** builtins (they fail with an "undefined variable" / rustc error):
`int`, `float`, `sorted`, `max`, `min`, `zip`, `reversed`, `type`.

Methods (called with `.`; see [UFCS](#ufcs) for calling free functions this
way):

- Strings: `.upper()`, `.lower()`, `.strip()`, `.split(sep)`,
  `.replace(a, b)`, `.startswith(p)`, `.len()`; `", ".join(list)`
- Lists: `.append(x)`, `.pop()`, `.len()`; membership is `x in xs`. There is
  no `.sort()`, `.reverse()`, `.push()` or `.contains()`.
- Dicts: `.keys()`, `.values()`, `.items()`, `.get(k)`, `k in d`,
  `d[k] = v`, `len(d)`

---

## 4. Advanced indexing

```python
vals[1:3]                      # slice
vals[-1]                       # negative index
vals[::2]                      # step
vals[::-1]                     # reverse
vals[[True, False, True]]      # boolean mask
vals[[0, 2, 4]]                 # gather — select elements at given indices
"hello"[1:3]                   # strings slice the same way -> "el"
players[.height > 180]         # field mask: records whose field passes the test
```

### Gathering a field across a collection of records

```python
data Reading(t, value)
readings = [Reading(1, 10.0), Reading(2, 20.0)]
readings..value                # [10.0, 20.0]
```

---

## 5. Optional values & error handling

```python
user?.address?.city ?? "unknown"     # optional chaining + None-coalescing
```

### Result, Ok/Err, and `?`

```python
def half(n):
    if n % 2 == 0:
        return Ok(n / 2)
    else:
        return Err("odd number")

def compute(n):
    v = half(n)?              # propagates Err(...) to the caller
    return Ok(v + 1)

match compute(10):
    case Ok(v):
        print("ok:", v)       # ok: 6.0
    case Err(e):
        print("err:", e)
```

`Ok(...)`/`Err(...)` construction and matching (`case Ok(v):` / `case Err(e):`)
work correctly in both backends. The `?` postfix operator, which propagates an
`Err` up to the caller as shown above, is fully working under `craw run`. In
the interpreter, `?` on an `Err(...)` value still aborts the whole program
with a runtime panic instead of gracefully returning that `Err` from the
enclosing function — so a program using `?` for control flow (not just
constructing/matching `Ok`/`Err` directly) should be run with `craw`, not
`crawler`.

---

## 6. Object orientation & types

### Data classes

```python
data Person(name, age)
p = Person("Amy", 30)
print(p.name, p.age)
p.age = 31                     # fields are assignable
print(p)                       # Person(Amy, 31)

data User(name, id=0)          # field defaults
User("a").id                   # 0
```

Field defaults are **transpiler only**: the interpreter panics with an
index-out-of-bounds error when a defaulted field is omitted.

> `struct`, `trait`, `impl`, and `class` are all broken. `struct`
> instantiation fails to compile (it reuses the dynamic `data`-class calling
> convention, which doesn't correspond to real Rust struct syntax), a
> body-less `trait` method signature confuses the lexer's raw-Rust-`fn`
> capture into swallowing the following `impl` block entirely, and a `class`
> method call panics with `AttributeError` under `craw run` and is rejected
> outright by the interpreter. Use `data` classes instead. `extend` (extension
> methods on existing types) isn't implemented either — it parses as an
> unrecognized macro block.

### UFCS

```python
def double(x) = x * 2
5.double()             # 10 — method lookup first, then free function
[1, 2].len()           # 2 — builtins work the same way
(a - b).abs()

data Box(fn)
b = Box(x => x + 1)
b.fn(10)                # 11 — a field that holds a callable is called directly
```

### Custom operators

Defining a `def` whose name is a backtick-quoted symbol makes that symbol
usable as an infix (two parameters) or prefix (one parameter) operator:

```python
def `⊕`(a, b) = a + b
def `√`(x) = sqrt(x)
1 ⊕ 2                    # 3
√64                      # 8.0
```

> **Transpiler only.** The interpreter fails with `Unknown operator`. The
> `operator ⊕(a, b) = ...` statement form is broken in both backends — use the
> backtick `def`.

Some Unicode operators are built in and work in both backends: `÷` (integer
division), `∑ [1, 2, 3]` (sum), `⊆` (subset), `∘` (composition).

---

## 7. Scoping & variables

```python
counter = 0
def incr():
    global counter
    counter = counter + 1
incr()
incr()
print(counter)          # 2
```

`global` works correctly in both backends. So does `nonlocal` (binding to an
*enclosing function's* scope, as opposed to module scope):

```python
def make_counter():
    n = 0
    def bump():
        nonlocal n
        n = n + 1
        return n
    return bump

c = make_counter()
print(c())               # 1
print(c())               # 2
```

### Operators

```python
val ?? default          # None-coalescing
a is b                   # reference/pointer equality; `x is None` works
user?.field               # optional chaining
a ≈ b                     # approximate equality — relative tolerance 1e-9
a and b                  # short-circuit logical and
a or b                   # short-circuit logical or
not a
a in xs                  # membership
a notin xs               # one word — `a not in xs` is a parse error
```

> The custom-tolerance form shown in some older notes, `a ≈[1e-6] b`, is not
> real syntax — it's a parse error.

---

## 8. Modules

Imports work like Python's. A file is a module, a directory is a package
(its own code, if any, lives in `__init__.craw`):

```text
project/
├── main.craw
├── geometry.craw
└── shapes/
    ├── __init__.craw
    └── circle.craw
```

```python
# geometry.craw
# Shapes and measurements.
data Point(x, y)

def area(w, h):
    return w * h

def _helper():             # a leading underscore: left out of `import *`
    return 42
```

```python
# shapes/circle.craw
from ..geometry import Point           # relative: from the package's parent

def origin():
    return Point(0, 0)
```

```python
# main.craw
import geometry                        # geometry.area(...)
import shapes.circle                   # binds `shapes`: shapes.circle.origin()
import shapes.circle as circle         # or bind the module itself
from geometry import area, Point as P  # bind names directly
from geometry import (                 # parenthesized lists may span lines
    area as rect_area,
)

print(geometry.area(3, 4))             # 12
print(area(1, 2), P(0, 0))
match circle.origin():
    case geometry.Point(x, y):         # a qualified pattern, as in Python
        print(x, y)
```

- `import a, b`, `import a.b as m`, `from m import x as y`,
  `from m import *` (the names in `__all__` if the module defines it,
  otherwise every name not starting with `_`), `from pkg import submodule`,
  and relative imports `from . import x` / `from ..pkg import y` all mean
  what they mean in Python.
- **Resolution.** Absolute imports search the program's root (the entry
  file's directory), then each directory in `CRAWPATH`. A module inside a
  package imports its siblings relatively (`from . import sibling`).
- **Each module runs once**, before the first file that imports it, in
  dependency order. Circular imports are an error (Python allows them;
  Craw doesn't).
- **Each module has its own namespace, types included**: two modules can
  both define `helper` or `Point`, and `case geometry.Point(...)` only
  matches geometry's. A type still prints as its plain name (`Point(0, 0)`).
  Templates and macros are expanded before the program runs, so they are
  shared by every module.
- **Rust items** are imported with Rust's own `use`
  (`use std::collections::HashMap`). The older `from std.collections import
  HashMap` still works for `std`, `core` and `alloc`, with an editor
  warning and a one-click fix to the `use` form.
- Unlike Python, a module isn't a value (`m = geometry` is an error), and
  imports must be at the top level of a file.

`craw run`, `craw build`, `craw transpile` and `crawler` all link imports,
and `craw run` rebuilds when any imported module changes. Errors name the
file they come from:

```text
main.craw:3:22: error: module `geometry` has no top-level `volume`
main.craw:2:20: error: `shapes.square` is a module that isn't imported; add `import shapes.square`
b.craw:1:1: error: circular import: a.craw → b.craw → a.craw
```

In VS Code, go to definition, hover, completion (`geometry.`,
`shapes.circle.`, `from geometry import `, `from ..`), signature help,
inlay hints, find references and rename all work across files, and broken
imports are flagged as you type.

---

## 9. Rust interop

> **Transpiler only.** Inline Rust blocks/expressions are spliced verbatim
> into the generated Rust source and compiled by `rustc`, so they only run
> under `craw run`/`craw build`. The `crawler` interpreter cannot execute
> arbitrary Rust at runtime and silently evaluates any inline Rust block or
> expression to `0.0` instead. If a program uses Rust interop, run it with
> `craw`, not `crawler`.

### Inline Rust blocks

A `rust:` block used as an expression yields its final expression; a `rust:`
block used as a bare statement must consist of complete Rust statements
(every line `;`-terminated — a trailing bare expression is a rustc error).

```python
cores = rust:
    let t = std::thread::available_parallelism().unwrap().get();
    t as i64

rust:
    let greeting = "hi";
    println!("{greeting}");
```

The value a block or escape hands back must be a type the runtime can convert:
`i64`, `f64`, `bool`, `String`, `usize`, `Vec<_>` or `HashMap<String, _>`. An
untyped integer literal (`let t = 42;`) is inferred as `i32` and fails to
compile — write `let t: i64 = 42;` or `42i64`.

### Inline Rust expressions

```python
val = ®(40i64 + 2)          # 42
val = ⚙(2.5f64 * 2.0)       # 5.0
val = 🦀(String::from("hi"))
🦀 println!("from rust");   # line form: splices one Rust statement
```

### Top-level Rust items

Lines starting with `use`, `extern`, `let` or `fn` at column 0 are captured
verbatim as Rust. A top-level `let` is reachable from escapes:

```python
use std::collections::HashMap;
let LIMIT: i64 = 5;
print(®(LIMIT * 2))         # 10
```

A top-level `fn` is compiled, but it **cannot be called from Craw code** —
neither directly nor through `®(...)` — because the transpiler treats the name
as an undefined Craw variable. Keep native helpers inside `rust:` blocks for
now.

### Templates

`template` defines a compile-time, textual macro that a `name arg* : body`
invocation expands into inline before the rest of the program is compiled. It
exists so a Craw program can introduce its own block-style statements —
something like a `repeat N:` loop or an `unless` — without adding a new
keyword to the compiler.

```python
template repeat(count, body):
    i = 0
    while i < count:
        body
        i = i + 1

repeat 3:
    print("hello")               # prints "hello" three times
```

Each `body`/`else_body`-style parameter binds to the whole indented block passed
at the call site; referencing that parameter as a statement inline-expands the
block at that point in the template. A template can also be invoked with one or
more named branches, each contributing its own block:

```python
template when(cond, then_body, branch_kw, else_body):
    matched = cond
    if matched:
        then_body
    if not matched:
        else_body

when x > 0:
    print("positive")
otherwise:
    print("non-positive")
```

The branch keyword (`otherwise` above) is itself passed positionally to the
template — `when` declares a `branch_kw` parameter to receive it even though
this template doesn't use it — so a template's parameter list must account for
every argument, keyword, and body block a call site supplies, in order, or
transpilation fails with an arity-mismatch error naming the template.
Substitution is purely textual (parameters are not hygienic), so avoid template
parameter names that collide with names used at the call site.

### Macros

`macro` is the same textual expansion invoked with call syntax, `@name(args)`,
instead of a block header. It works in both backends:

```python
macro check(cond, msg):
    if not cond:
        print("FAIL:", msg)
    if cond:
        print("OK:", msg)

@check(1 < 2, "math")            # OK: math
```

`@name` on the line before a `def` is *not* a decorator — it's a call to a
macro named `name`, and fails if none is defined (so `@memo`, `@pure` etc. do
nothing useful).

---

## 10. Shell & scripting

```python
output = `echo hello`
```

Backtick expressions run a subprocess and return its stdout as a String.

> Transpiler only — the interpreter doesn't support shell/backtick
> expressions at all (`Interpreter does not yet support expression:
> Shell(...)`).

---

## 11. Operators reference

| Operator                    | Meaning                                    |
| ---------------------------- | ------------------------------------------- |
| `+` `-` `*` `/`              | Arithmetic (`/` is always float division)   |
| `//` `÷`                     | Integer division                            |
| `%`                          | Remainder (sign follows the dividend)       |
| `**`                         | Power (see precedence note below)           |
| `.+` `.-` `.*` `./`          | Element-wise (broadcasting)                 |
| `f.(xs)`                     | Broadcast call                              |
| `==` `!=` `<` `<=` `>` `>=`  | Comparison                                  |
| `≈`                          | Approximate equality (relative, 1e-9)       |
| `and` `or` `not`             | Logical (short-circuit)                     |
| `in` `notin`                 | Membership                                  |
| `??`                         | None-coalescing                             |
| `?.`                         | Optional chaining                           |
| `?` (postfix)                | `Err` propagation (transpiler only)         |
| `|>` `|?>` `|*>` `<|`        | Pipe / None-aware / splat / backward pipe   |
| `∘` `>>` `<<`                | Function composition                        |
| `f$(...)`                    | Partial application (`_` marks a hole)      |
| `a..b`                       | Inclusive range                             |
| `is`                         | Reference/pointer equality                  |
| `..field`                    | Gather a field across a collection          |
| `xs[.field > v]`             | Field mask                                  |
| `|` `&` `⊆`                  | Set union / intersection / subset           |
| `∑`                          | Sum (prefix)                                |

> **Precedence of `**`** differs from Python: it is left-associative and sits
> at the same level as `*`, so `2 ** 3 ** 2` is `64` (not `512`),
> `2 * 3 ** 2` is `36` (not `18`), and `-2 ** 2` is `4` (not `-4`).
> Parenthesise.

`±` and `<unit>` also lex, but nothing backs them yet.
