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

## Installation

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (edition 2024, so a recent
  toolchain — `rustup update` if `cargo build` complains about the edition)
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
```

### VS Code extension

A packaged extension is included at `craw-0.1.0.vsix`. Install it from VS Code
with:

```sh
code --install-extension craw-0.1.0.vsix
```

To build the extension from source instead, see `build_extension.sh` /
`build_extension.ps1` in the repo root.

# Language at a glance

## 1. Basic types & literals

### Scalars

- `Int` (64-bit), `Float` (64-bit), `String` (UTF-8), `Bool` (`True`/`False`),
  `None`

### Collections

- List: `[1, 2, 3]` (mutable)
- Tuple: `(1, 2, 3)` (immutable)
- Dict: `{"key": value}` — any hashable key type, e.g. `Dict[Int, String]`,
  `Dict[Shape, Float]`
- Set: `Set(1, 2, 3)` — Frozenset: `Frozenset(1, 2)` — Multiset:
  `Multiset(1, 1, 2)`

### Arrays

- N-dimensional: `[1, 2; 3, 4]` — semicolons delimit rows, commas delimit
  columns

### String literals

```
f"result is {x * 2}"        # interpolation
f"hello {name.upper()}"

msg = """
  hello
  world
"""                          # triple-quoted, leading whitespace dedented automatically
```

### Numeric suffixes

```
x = 1.000000000000001p       # BigDecimal (arbitrary precision)
y = 123456789012345678d       # exact integer
```

---

## 2. Control flow

### Conditionals

```
if cond: body
else: body
```

### Loops

```
while cond: body
for item in collection: body
```

### Pattern matching

```
match value:
  case Point(0, y):           "on Y axis"
  case (1, x, _):             "tuple starts with 1"
  case n if n > 0:            "positive number"   # guard clause
  case n if n < 0:            "negative number"
  case sorted([first, *_]):   f"min is {first}"   # view pattern
  case abs(n) if n < 1e-9:    "near zero"         # view pattern + guard
  case _:                     "default"
```

### Destructuring in for loops

```
for (x, y) in points:        # tuple — non-matching shapes skipped
  plot(x, y)

for {name, age} in records:  # struct fields
  print(name)
```

### Destructuring assignment

```
(a, b, c) = some_tuple
{name, age} = person          # name and age bound directly in scope
```

### For-comprehensions (Scala-style)

Desugar to flatMap / filter / map over any monad-like type.

```
result = for x <- items
             if x > 0
             y <- transform(x)
         yield x + y
```

### Resource management

```
with open("file.txt") as f:
  data = f.read()             # f.close() called automatically on exit
```

---

## 3. Functional programming

### Lambdas

```
x => x + 1                   # canonical short form
lambda x:                    # multi-line form only
  y = x * 2
  y + 1
```

### Implicit lambdas & partial application

```
(_ + 1)                       # implicit lambda — shorthand for x => x + 1
add$(1, _)                    # partial application — _ is the hole in both forms
```

### Piping

```
data |> filter(.age > 18) |> select(.name)
data |?> transform            # None-aware pipe: skips if LHS is None
```

### Composition

```
f ∘ g                         # f(g(x))
```

### Broadcasting

```
x .+ y                        # element-wise add
x .- y                        # element-wise subtract
x .* y                        # element-wise multiply
x ./ y                        # element-wise divide
```

### Generators

```
gen naturals():
  n = 0
  while True:
    yield n
    n += 1

take(10, naturals())          # lazy — no intermediate allocation
```

### Memoization

```
@memo
fn fib(n: Int) -> Int:
  if n <= 1: n else fib(n-1) + fib(n-2)
```

Compiler verifies the function is pure before allowing `@memo`.

### Pure annotation (optional)

```
@pure
fn add(a: Int, b: Int) -> Int: a + b
```

Compiler checks: no global mutation, no IO, no non-determinism. Enables safe
auto-parallelism.

### Algebraic effects

```
effect Log:
  fn log(msg: String) -> None

effect Random:
  fn next() -> Float

fn compute() with Log, Random:
  Log.log("starting")
  Random.next() * 42
```

Effects compose without monadic plumbing.

---

## 5. Advanced indexing

```
x[start:stop:step]            # Python-style slicing, negative indices supported
data[[True, False, True]]     # boolean mask
data[.age > 18]               # expression mask
data[[0, 2, 4]]               # gather — select elements at given indices
```

---

## 6. Type system

### Sum types / tagged unions

```
type Shape =
  | Circle(r: Float)
  | Rect(w: Float, h: Float)
  | Point

match s:
  case Circle(r):  pi * r**2
  case Rect(w, h): w * h
  case Point:      0.0          # exhaustiveness checked at compile time
```

### Refinement types

```
type PosInt = Int where _ > 0
type Prob   = Float where 0.0 <= _ <= 1.0

fn sqrt(x: Float where _ >= 0.0) -> Float: ...
```

Checked at compile time where provable, runtime otherwise.

### Opaque types (Scala-style)

Zero-cost type aliases that prevent accidental mixing of structurally identical
types.

```
opaque type UserId = String
opaque type PostId = String

fn get_post(id: PostId) -> Post: ...

uid: UserId = UserId("u-123")
get_post(uid)                 # compile error: UserId is not PostId
```

### Result type & error propagation

```
fn read(path: String) -> Result[String, IOError]:
  content = fs.read(path)?    # ? propagates error to caller
  Ok(content)
```

### Optional chaining

```
user?.address?.city ?? "unknown"
```

### Row polymorphism

```
fn greet(e: { name: String }) -> String:
  f"hello {e.name}"

# Accepts any struct/record with a name field — Person, Animal, Robot, etc.
```

### Dimension-typed arrays (optional)

```
fn dot(a: Arr[N, M], b: Arr[M, P]) -> Arr[N, P]: ...
# Shape mismatches caught at compile time
```

### Match types (Scala-style)

Types computed by matching on other types.

```
type Element[T] = T match:
  case List[a] => a
  case _       => T

fn head(xs: List[T]) -> Element[List[T]]: xs[0]
```

---

## 7. Object orientation & types

### Data classes

```
data Person(name: String, age: Int)
```

### Structs

```
struct Point:
  x: Int
  y: Int
```

### Traits & impl (Rust-style)

```
trait Area:
  fn area(self) -> Float

impl Area for Circle:
  fn area(self) -> Float: pi * self.r**2
```

### Extension methods (Scala-style)

Add methods to existing types without modifying their definition.

```
extend Int:
  fn factorial() -> Int:
    if self <= 1: 1 else self * (self - 1).factorial()

5.factorial()                 # => 120
```

### UFCS

```
obj.func(args)                # equivalent to func(obj, args)
                              # method lookup first, then free function
free(func, obj, args)         # explicit free-function disambiguation
```

---

## 8. Scoping & variables

### Lexical scope

Standard scoping with `global` and `nonlocal` keywords.

### where clause

```
result = f(x, y) where:
  x = compute()               # may reference outer scope
  y = x * 2                   # sequential evaluation: y may reference x
```

### Operators

```
val ?? default                # None-coalescing
a is b                        # reference/pointer equality
user?.field                   # optional chaining
a ≈ b                         # approximate equality — |a-b|/max(|a|,|b|) < 1e-9
a ≈[1e-6] b                   # approximate equality with custom tolerance
```

### Context parameters (Scala given/using)

Declare a parameter received implicitly from the call context.

```
given precision: Precision = Precision(1e-9)

fn format(x: Float)(using prec: Precision) -> String:
  x.toFixed(prec.digits)

format(3.14159)               # picks up given Precision automatically
format(3.14159)(using Precision(1e-3))  # explicit override
```

---

## 9. Scientific & numeric

### Units of measure

```
g   = 9.8<m/s²>
t   = 3.0<s>
v   = g * t                   # inferred :: <m/s>
bad = g + t                   # compile error: m/s² + s is dimensionally inconsistent
```

### Approximate equality

```
a ≈ b                         # relative tolerance 1e-9
a ≈[1e-6] b                   # custom tolerance
```

### Uncertainty arithmetic

```
x = 9.8 ± 0.01
y = 3.0 ± 0.05
z = x * y                     # uncertainty propagated automatically via interval arithmetic
```

### Symbolic differentiation

```
fn f(x: Float) -> Float: x**3 + 2*x

df  = diff(f, x)              # first derivative
d2f = diff(f, x, 2)           # second derivative

df(3.0)                       # => 29.0
```

### Named & optional arguments

```
fn solve(a: Float, b: Float, tol: Float = 1e-9, max_iter: Int = 1000) -> Float:
  ...

solve(1.0, 2.0)               # tol and max_iter use defaults
solve(1.0, 2.0, tol=1e-6)     # keyword — remaining defaults apply
```

### Varargs & splat

```
fn sum(*xs: Int) -> Int: xs |> reduce((_ + _))

nums = [1, 2, 3]
sum(*nums)                    # splat a collection into varargs
```

---

## 10. Rust interop

### Inline Rust blocks

```
rust:
  let x: u64 = 42;
  x * 2
```

### Inline Rust expressions

```
val = ®(rust_expr)
val = ⚙(rust_expr)
val = 🦀(rust_expr)
```

### Templates

`template` defines a compile-time, textual macro that a `name arg* : body`
invocation expands into inline before the rest of the program is compiled. It
exists so a Craw program can introduce its own block-style statements — the
language's `if` has no `else`, so `template` is how you'd build one — without
adding a new keyword to the compiler.

```
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

```
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

---

## 11. Concurrency

### Async / await with structured concurrency

```
async fn fetch(url: String) -> String: ...

async fn main():
  async with TaskGroup() as tg:
    a = tg.spawn(fetch(url1))
    b = tg.spawn(fetch(url2))
  print(a.result, b.result)   # all tasks complete before this line
```

No fire-and-forget spawning — all tasks are scoped to their TaskGroup.

### Data-parallel for loop (optional)

```
@pure fn process(x: Float) -> Float: x**2 + 1.0

par for x in large_dataset:
  results.append(process(x))
```

`@pure` annotation allows the compiler to verify safety automatically.

---

## 12. Metaprogramming

### Compile-time evaluation (optional)

```
comptime fn primes(n: Int) -> List[Int]: ...

SMALL_PRIMES = comptime primes(100)   # evaluated at compile time
```

### Hygienic macros (optional)

```
macro my_query(expr):
  # transforms AST nodes — LINQ and similar features can be defined here
  ...
```

---

## 13. Shell & scripting

### Subprocess syntax

```
output = `git log --oneline`
branch = `git rev-parse --abbrev-ref HEAD`
```

Backtick expressions run a subprocess and return its stdout as a String.

---

## 14. Operators reference

| Operator                    | Meaning                               |
| --------------------------- | ------------------------------------- |
| `+` `-` `*` `/`             | Arithmetic                            |
| `÷`                         | Integer division                      |
| `%`                         | Modulo                                |
| `**`                        | Power                                 |
| `.+` `.-` `.*` `./`         | Element-wise (broadcasting)           |
| `==` `!=` `<` `<=` `>` `>=` | Comparison                            |
| `≈` `≈[ε]`                  | Approximate equality                  |
| `and` `or` `not`            | Logical                               |
| `??`                        | None-coalescing                       |
| `?.`                        | Optional chaining                     |
| `                           | >`                                    |
| `                           | ?>`                                   |
| `∘` `>>`                    | Function composition                  |
| `is`                        | Reference equality                    |
| `?`                         | Error propagation (in Result context) |
| `±`                         | Uncertainty literal                   |
| `<unit>`                    | Unit-of-measure annotation            |
