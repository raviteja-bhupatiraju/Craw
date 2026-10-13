import * as fs from 'fs';
import * as path from 'path';
import { beforeAll, describe, expect, it } from 'vitest';
import * as oniguruma from 'vscode-oniguruma';
import * as vsctm from 'vscode-textmate';

const root = path.join(__dirname, '..', '..', '..');
const repoRoot = path.join(root, '..', '..');
const grammarPath = path.join(root, 'syntaxes', 'craw.tmLanguage.json');

interface Token {
  text: string;
  scopes: string[];
}

let grammar: vsctm.IGrammar;

beforeAll(async () => {
  const wasm = fs.readFileSync(path.join(root, 'node_modules', 'vscode-oniguruma', 'release', 'onig.wasm'));
  await oniguruma.loadWASM(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength));
  const registry = new vsctm.Registry({
    onigLib: Promise.resolve({
      createOnigScanner: (sources: string[]) => new oniguruma.OnigScanner(sources),
      createOnigString: (s: string) => new oniguruma.OnigString(s),
    }),
    // Only Craw is available; `source.rust` (and anything else) is stubbed
    // out, as it would be for a user without a Rust grammar installed.
    loadGrammar: async (scopeName: string) =>
      scopeName === 'source.craw'
        ? vsctm.parseRawGrammar(fs.readFileSync(grammarPath, 'utf8'), grammarPath)
        : null,
  });
  const g = await registry.loadGrammar('source.craw');
  if (!g) throw new Error('failed to load source.craw');
  grammar = g;
});

/** Tokenizes `src` (multi-line) into one token array per line. */
function tokenize(src: string): Token[][] {
  let state = vsctm.INITIAL;
  return src.split('\n').map((line) => {
    const r = grammar.tokenizeLine(line, state);
    state = r.ruleStack;
    return r.tokens.map((t) => ({ text: line.slice(t.startIndex, t.endIndex), scopes: t.scopes }));
  });
}

/**
 * Scope stack of the token covering the first char of the `occurrence`-th
 * appearance of `needle` in `src`.
 */
function scopesAt(src: string, needle: string, occurrence = 0): string[] {
  let idx = -1;
  for (let i = 0; i <= occurrence; i++) {
    idx = src.indexOf(needle, idx + 1);
    if (idx < 0) throw new Error(`"${needle}" (#${occurrence}) not found in ${JSON.stringify(src)}`);
  }
  const before = src.slice(0, idx).split('\n');
  const lineNo = before.length - 1;
  const col = before[lineNo].length;
  const lines = tokenize(src);
  let pos = 0;
  for (const tok of lines[lineNo]) {
    if (col >= pos && col < pos + tok.text.length) return tok.scopes;
    pos += tok.text.length;
  }
  throw new Error(`no token at ${lineNo}:${col}`);
}

/** The text of the token covering the start of `needle`. */
function tokenText(src: string, needle: string, occurrence = 0): string {
  let idx = -1;
  for (let i = 0; i <= occurrence; i++) idx = src.indexOf(needle, idx + 1);
  const before = src.slice(0, idx).split('\n');
  const lineNo = before.length - 1;
  const col = before[lineNo].length;
  let pos = 0;
  for (const tok of tokenize(src)[lineNo]) {
    if (col >= pos && col < pos + tok.text.length) return tok.text;
    pos += tok.text.length;
  }
  throw new Error('no token');
}

/** Whether some scope in the stack is `prefix` or a sub-scope of it. */
function has(scopes: string[], prefix: string): boolean {
  return scopes.some((s) => s === prefix || s.startsWith(prefix + '.'));
}

function expectScope(src: string, needle: string, prefix: string, occurrence = 0): void {
  const scopes = scopesAt(src, needle, occurrence);
  expect(has(scopes, prefix), `${needle} in ${JSON.stringify(src)} -> ${scopes.join(' ')}`).toBe(true);
}

function expectNoScope(src: string, needle: string, prefix: string, occurrence = 0): void {
  const scopes = scopesAt(src, needle, occurrence);
  expect(has(scopes, prefix), `${needle} in ${JSON.stringify(src)} -> ${scopes.join(' ')}`).toBe(false);
}

/** Keyword spellings from `read_ident_or_keyword` in src/lexer.rs. */
function lexerKeywords(): string[] {
  const lexer = fs.readFileSync(path.join(repoRoot, 'src', 'lexer.rs'), 'utf8');
  const start = lexer.indexOf('fn read_ident_or_keyword');
  const end = lexer.indexOf('fn read_number');
  const body = lexer.slice(start, end);
  return [...body.matchAll(/"(\w+)" => Token::/g)].map((m) => m[1]);
}

/** Names from `crate::transpiler::BUILTINS`. */
function transpilerBuiltins(): string[] {
  const src = fs.readFileSync(path.join(repoRoot, 'src', 'transpiler.rs'), 'utf8');
  const start = src.indexOf('pub const BUILTINS');
  const body = src.slice(start, src.indexOf('];', start));
  return [...body.matchAll(/\("([^"]+)",/g)].map((m) => m[1]);
}

describe('craw.tmLanguage.json', () => {
  it('is valid JSON with scopeName source.craw', () => {
    const raw = JSON.parse(fs.readFileSync(grammarPath, 'utf8'));
    expect(raw.scopeName).toBe('source.craw');
  });

  it('maps meta.embedded.block.rust to rust in package.json', () => {
    const pkg = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'));
    const entry = pkg.contributes.grammars.find((g: any) => g.scopeName === 'source.craw');
    expect(entry.embeddedLanguages['meta.embedded.block.rust']).toBe('rust');
  });
});

describe('keywords', () => {
  const keywords = lexerKeywords();

  it('found the lexer keyword table', () => {
    expect(keywords.length).toBeGreaterThan(40);
    expect(keywords).toContain('def');
    expect(keywords).toContain('notin');
  });

  it('highlights every word the lexer treats as a keyword', () => {
    for (const kw of keywords) {
      const src = `a ${kw} b`;
      const scopes = scopesAt(src, kw);
      const ok = has(scopes, 'keyword') || has(scopes, 'storage');
      expect(ok, `${kw} -> ${scopes.join(' ')}`).toBe(true);
      expect(tokenText(src, kw)).toBe(kw);
    }
  });

  it.each([
    ['if', 'keyword.control'],
    ['else', 'keyword.control'],
    ['then', 'keyword.control'],
    ['while', 'keyword.control'],
    ['for', 'keyword.control'],
    ['break', 'keyword.control'],
    ['continue', 'keyword.control'],
    ['return', 'keyword.control'],
    ['yield', 'keyword.control'],
    ['match', 'keyword.control'],
    ['case', 'keyword.control'],
    ['where', 'keyword.control'],
    ['import', 'keyword.control.import'],
    ['from', 'keyword.control.import'],
    ['as', 'keyword.control.import'],
    ['select', 'keyword.control.query'],
    ['orderby', 'keyword.control.query'],
    ['and', 'keyword.operator.logical'],
    ['or', 'keyword.operator.logical'],
    ['not', 'keyword.operator.logical'],
    ['notin', 'keyword.operator.logical'],
    ['is', 'keyword.operator.logical'],
    ['in', 'keyword.operator.logical'],
    ['to', 'keyword.operator.range'],
    ['until', 'keyword.operator.range'],
    ['global', 'storage.modifier'],
    ['copyclosure', 'storage.modifier'],
    ['lambda', 'storage.type.function.lambda'],
    ['def', 'storage.type.function'],
    ['gen', 'storage.type.function'],
    ['template', 'storage.type.function'],
    ['macro', 'storage.type.function'],
    ['operator', 'storage.type.function'],
    ['data', 'storage.type.class'],
    ['class', 'storage.type.class'],
    ['struct', 'storage.type.struct'],
    ['trait', 'storage.type.trait'],
    ['impl', 'storage.type.impl'],
  ])('%s is %s', (kw, scope) => {
    expectScope(`x ${kw} y`, kw, scope);
  });

  it.each(['try', 'async', 'await', 'except', 'finally', 'raise', 'pass', 'elif', 'let', 'fn', 'type', 'enum', 'cases', 'del', 'assert'])(
    '%s (not a lexer keyword) is a plain identifier',
    (word) => {
      const src = `x = ${word} + 1`;
      expectNoScope(src, word, 'keyword');
      expectNoScope(src, word, 'storage');
      expectScope(src, word, 'variable.other.craw');
    },
  );

  it('does not highlight keywords inside identifiers', () => {
    expectScope('iffy = format + index', 'iffy', 'variable.other.craw');
    expectScope('iffy = format + index', 'format', 'variable.other.craw');
    expectScope('iffy = format + index', 'index', 'variable.other.craw');
    expectScope('définition = 1', 'définition', 'variable.other.craw');
  });

  it('treats a standalone λ as lambda but λx as an identifier', () => {
    expectScope('f = λ x: x', 'λ', 'storage.type.function.lambda');
    expectScope('λx = 1', 'λx', 'variable.other.craw');
  });

  it('only rust: (with the colon) is the passthrough keyword', () => {
    expectScope('rust = 1', 'rust', 'variable.other.craw');
    expectScope('x = rust:', 'rust', 'keyword.other.passthrough.craw');
  });
});

describe('constants', () => {
  it.each(['True', 'False', 'None'])('%s is constant.language', (c) => {
    expectScope(`x = ${c}`, c, 'constant.language.craw');
  });

  it.each(['true', 'false', 'none'])('lowercase %s is an ordinary identifier', (c) => {
    expectNoScope(`x = ${c}`, c, 'constant.language');
  });
});

describe('comments', () => {
  it('scopes # to end of line', () => {
    const src = 'x = 1 # a comment';
    expectScope(src, '#', 'punctuation.definition.comment.craw');
    expectScope(src, 'a comment', 'comment.line.number-sign.craw');
    expectNoScope(src, 'x', 'comment');
  });

  it('highlights codetags inside comments', () => {
    const src = '# TODO: fix FIXME and XXX but not TODOS';
    expectScope(src, 'TODO', 'keyword.codetag.craw');
    expectScope(src, 'FIXME', 'keyword.codetag.craw');
    expectScope(src, 'XXX', 'keyword.codetag.craw');
    expectNoScope(src, 'TODOS', 'keyword.codetag');
    expectNoScope('TODO = 1', 'TODO', 'keyword.codetag');
  });

  it('a # inside a string is not a comment', () => {
    const src = 'x = "a # b" + \'c # d\'';
    expectNoScope(src, '# b', 'comment');
    expectNoScope(src, '# d', 'comment');
    expectScope(src, '# b', 'string.quoted.double.craw');
  });

  it('a # inside an f-string is not a comment', () => {
    expectNoScope('x = f"#{n}"', '#', 'comment');
  });
});

describe('strings', () => {
  it('double and single quoted strings', () => {
    expectScope('x = "hi"', 'hi', 'string.quoted.double.craw');
    expectScope("x = 'hi'", 'hi', 'string.quoted.single.craw');
    expectScope('x = "hi"', '"', 'punctuation.definition.string.begin.craw');
  });

  it('escapes', () => {
    const src = 'x = "a\\n\\"b\\\\" + \'\\t\\\'\'';
    expectScope(src, '\\n', 'constant.character.escape.craw');
    expectScope(src, '\\"', 'constant.character.escape.craw');
    expectScope(src, '\\\\', 'constant.character.escape.craw');
    expectScope(src, '\\t', 'constant.character.escape.craw');
    expectScope(src, "\\'", 'constant.character.escape.craw');
    // The escaped quote does not close the string.
    expectScope(src, 'b', 'string.quoted.double.craw');
  });

  it('keywords inside strings are not keywords', () => {
    expectNoScope('x = "if def"', 'if', 'keyword');
  });

  it('triple-quoted strings span lines', () => {
    const src = 'x = """line one\nif " still\nend"""\ny = 1';
    expectScope(src, 'line one', 'string.quoted.double.block.craw');
    expectScope(src, 'if', 'string.quoted.double.block.craw');
    expectScope(src, 'end', 'string.quoted.double.block.craw');
    expectNoScope(src, 'y', 'string');
    expectScope("x = '''a\nb'''", 'b', 'string.quoted.single.block.craw');
  });

  it('"" is an empty string, not the start of a triple-quoted one', () => {
    const src = 'x = ""\ny = 1';
    expectNoScope(src, 'y', 'string');
  });

  it('ordinary strings may also span lines (the lexer does not stop at newlines)', () => {
    const src = 'x = "a\nb"\ny = 1';
    expectScope(src, 'b', 'string.quoted.double.craw');
    expectNoScope(src, 'y', 'string');
  });

  it('backtick strings (shell commands)', () => {
    expectScope('out = `git log`', 'git log', 'string.interpolated.backtick.craw');
  });
});

describe('f-strings', () => {
  const src = 'msg = f"hello {name.upper() + 1} {{lit}} done"';

  it('scopes the prefix and literal text', () => {
    expectScope(src, 'f"', 'storage.type.string.craw');
    expectScope(src, 'hello', 'string.interpolated.craw');
    expectScope(src, 'hello', 'meta.fstring.craw');
    expectScope(src, 'done', 'string.interpolated.craw');
  });

  it('interpolation braces are punctuation', () => {
    expectScope(src, '{name', 'punctuation.section.interpolation.begin.craw');
    expectScope(src, '} {{', 'punctuation.section.interpolation.end.craw');
  });

  it('the interpolated expression gets expression scopes', () => {
    expectScope(src, 'name', 'meta.embedded.expression.craw');
    expectNoScope(src, 'name', 'string');
    expectScope(src, 'upper', 'entity.name.function.member.craw');
    expectScope(src, '+', 'keyword.operator.arithmetic.craw');
    expectScope(src, '1', 'constant.numeric.integer.craw');
  });

  it('{{ and }} are escapes', () => {
    expectScope(src, '{{', 'constant.character.escape.craw');
    expectScope(src, '}}', 'constant.character.escape.craw');
    expectNoScope(src, 'lit', 'meta.embedded.expression');
  });

  it('nested braces inside an interpolation are balanced', () => {
    const s = 'x = f"{ {1: 2}[1] } tail"';
    expectScope(s, 'tail', 'string.interpolated.craw');
    expectScope(s, '2', 'meta.embedded.expression.craw');
  });

  it('ends at the first double quote, even inside an interpolation', () => {
    const s = 'x = f"{a"\ny = 1';
    expectNoScope(s, 'y', 'meta.fstring');
    expectNoScope(s, 'y', 'string');
  });

  it('has no backslash escapes (the lexer copies f-string text verbatim)', () => {
    const s = 'x = f"a\\" + b';
    expectScope(s, 'b', 'variable.other.craw');
  });

  it('f"" only starts where a token can start', () => {
    expectScope('elf"x"', 'elf', 'variable.other.craw');
    expectScope('elf"x"', 'x', 'string.quoted.double.craw');
    expectNoScope("x = f'y'", 'y', 'meta.fstring');
  });
});

describe('numbers', () => {
  it.each([
    ['42', 'constant.numeric.integer.craw'],
    ['1_000_000', 'constant.numeric.integer.craw'],
    ['0xFF_FF', 'constant.numeric.integer.hexadecimal.craw'],
    ['0o17', 'constant.numeric.integer.octal.craw'],
    ['0b1010', 'constant.numeric.integer.binary.craw'],
    ['3.14', 'constant.numeric.float.craw'],
    ['1_000.5', 'constant.numeric.float.craw'],
    ['1e9', 'constant.numeric.float.craw'],
    ['2.5E-3', 'constant.numeric.float.craw'],
  ])('%s is %s', (num, scope) => {
    const src = `x = ${num} + y`;
    expectScope(src, num, scope);
    expect(tokenText(src, num)).toBe(num);
  });

  it('1..5 is a range, not a float', () => {
    const src = 'r = 1..5';
    expect(tokenText(src, '1')).toBe('1');
    expectScope(src, '..', 'keyword.operator.range.craw');
    expectScope(src, '5', 'constant.numeric.integer.craw');
  });

  it('1e_x is an integer followed by an identifier', () => {
    expect(tokenText('x = 1e_x', '1')).toBe('1');
  });

  it('digits inside identifiers are not numbers', () => {
    expectScope('x1 = y2', 'x1', 'variable.other.craw');
  });
});

describe('operators', () => {
  it.each([
    // arithmetic
    ['+', 'keyword.operator.arithmetic'],
    ['-', 'keyword.operator.arithmetic'],
    ['*', 'keyword.operator.arithmetic'],
    ['/', 'keyword.operator.arithmetic'],
    ['%', 'keyword.operator.arithmetic'],
    ['**', 'keyword.operator.arithmetic'],
    ['//', 'keyword.operator.arithmetic'],
    ['÷', 'keyword.operator.arithmetic'],
    ['±', 'keyword.operator.arithmetic'],
    ['.+', 'keyword.operator.arithmetic.broadcast'],
    ['.-', 'keyword.operator.arithmetic.broadcast'],
    ['.*', 'keyword.operator.arithmetic.broadcast'],
    ['./', 'keyword.operator.arithmetic.broadcast'],
    ['.%', 'keyword.operator.arithmetic.broadcast'],
    // comparison
    ['==', 'keyword.operator.comparison'],
    ['!=', 'keyword.operator.comparison'],
    ['<', 'keyword.operator.comparison'],
    ['>', 'keyword.operator.comparison'],
    ['<=', 'keyword.operator.comparison'],
    ['>=', 'keyword.operator.comparison'],
    ['≈', 'keyword.operator.comparison'],
    ['≠', 'keyword.operator.comparison'],
    ['≤', 'keyword.operator.comparison'],
    ['≥', 'keyword.operator.comparison'],
    ['∈', 'keyword.operator.comparison'],
    ['∉', 'keyword.operator.comparison'],
    ['.==', 'keyword.operator.comparison.broadcast'],
    ['.!=', 'keyword.operator.comparison.broadcast'],
    ['.<=', 'keyword.operator.comparison.broadcast'],
    ['.>', 'keyword.operator.comparison.broadcast'],
    // assignment
    ['=', 'keyword.operator.assignment'],
    ['+=', 'keyword.operator.assignment'],
    ['-=', 'keyword.operator.assignment'],
    ['*=', 'keyword.operator.assignment'],
    ['/=', 'keyword.operator.assignment'],
    ['**=', 'keyword.operator.assignment'],
    // pipes and composition
    ['|>', 'keyword.operator.pipe'],
    ['|*>', 'keyword.operator.pipe'],
    ['|**>', 'keyword.operator.pipe'],
    ['|?>', 'keyword.operator.pipe'],
    ['|?*>', 'keyword.operator.pipe'],
    ['|?**>', 'keyword.operator.pipe'],
    ['<|', 'keyword.operator.pipe'],
    ['<*|', 'keyword.operator.pipe'],
    ['<**|', 'keyword.operator.pipe'],
    ['<?|', 'keyword.operator.pipe'],
    ['<*?|', 'keyword.operator.pipe'],
    ['<**?|', 'keyword.operator.pipe'],
    ['>>', 'keyword.operator.pipe.compose'],
    ['<<', 'keyword.operator.pipe.compose'],
    ['..*>', 'keyword.operator.pipe.compose'],
    ['..**>', 'keyword.operator.pipe.compose'],
    ['..?>', 'keyword.operator.pipe.compose'],
    ['∘', 'keyword.operator.pipe.compose'],
    // misc
    ['..', 'keyword.operator.range'],
    ['??', 'keyword.operator.logical'],
    ['$', 'keyword.operator.partial'],
    ['=>', 'keyword.operator.arrow'],
    ['->', 'keyword.operator.arrow'],
    ['?', 'keyword.operator.ternary'],
    ['~', 'keyword.operator.formula'],
    ['&', 'keyword.operator.bitwise'],
    ['|', 'keyword.operator.bitwise'],
    ['∩', 'keyword.operator.set'],
    ['∪', 'keyword.operator.set'],
    ['⊕', 'keyword.operator.custom'],
  ])('%s is %s (as one token)', (op, scope) => {
    const src = `a ${op} b`;
    expectScope(src, op, scope);
    expect(tokenText(src, op)).toBe(op);
  });

  it('..> is `..` then `>` (the lexer has no ..> operator)', () => {
    const src = 'f ..> g';
    expect(tokenText(src, '..')).toBe('..');
    expectScope(src, '>', 'keyword.operator.comparison');
  });

  it('.= is `.` then `=`', () => {
    const src = 'x .= 1';
    expect(tokenText(src, '.')).toBe('.');
    expectScope(src, '=', 'keyword.operator.assignment');
  });

  it('a bare ! (dropped by the lexer) is flagged', () => {
    expectScope('x = ! y', '!', 'invalid.illegal');
    // `.!` is no operator either (the parser rejects it); `.!=` still is.
    expectScope('z = a.!', '!', 'invalid.illegal');
    expectScope('z = a .!= b', '.!=', 'keyword.operator.comparison.broadcast');
  });

  it('operator sections are a single operator', () => {
    const src = 'f = reduce((+), xs) |> (÷)';
    expectScope(src, '(+', 'meta.operator-section.craw');
    expectScope(src, '+', 'keyword.operator.arithmetic');
    expectScope(src, '÷', 'meta.operator-section.craw');
    expectNoScope('f(-1)', '-', 'meta.operator-section');
    expectNoScope('f(*args)', '*', 'meta.operator-section');
    expectNoScope('xs[(.x > 1)]', '.', 'meta.operator-section');
  });
});

describe('definitions', () => {
  it('def name and parameters', () => {
    const src = 'def area(width, height=2, *rest):';
    expectScope(src, 'def', 'storage.type.function.craw');
    expectScope(src, 'area', 'entity.name.function.craw');
    expectScope(src, 'width', 'variable.parameter.craw');
    expectScope(src, 'height', 'variable.parameter.craw');
    expectScope(src, 'rest', 'variable.parameter.craw');
    expectScope(src, '2', 'constant.numeric.integer.craw');
    expectNoScope(src, '2', 'variable.parameter');
  });

  it('builtin types in annotations', () => {
    const src = 'def f(x: Int, ys: List[Float], d: Dict[String, Bool] = none) -> String:';
    expectScope(src, 'x', 'variable.parameter.craw');
    expectScope(src, 'Int', 'support.type.craw');
    expectScope(src, 'List', 'support.type.craw');
    expectScope(src, 'Float', 'support.type.craw');
    expectScope(src, 'Dict', 'support.type.craw');
    expectScope(src, 'Bool', 'support.type.craw');
    expectNoScope(src, 'Bool', 'variable.parameter');
    expectScope(src, 'd:', 'variable.parameter.craw');
    expectScope(src, 'String', 'support.type.craw', 1);
    expectScope(src, '->', 'keyword.operator.arrow.craw');
  });

  it('None as a return type is support.type', () => {
    expectScope('def f(x: Dynamic) -> None:', 'None', 'support.type.craw');
    expectScope('def f(x: Dynamic) -> None:', 'Dynamic', 'support.type.craw');
    expectScope('x = None', 'None', 'constant.language.craw');
  });

  it('default values with calls do not leak parameter scopes', () => {
    const src = 'def f(a=g(b, c), d):';
    expectScope(src, 'g', 'entity.name.function.call.craw');
    expectNoScope(src, 'c', 'variable.parameter');
    expectScope(src, 'd)', 'variable.parameter.craw');
  });

  it('destructuring parameters', () => {
    const src = 'def f((a, b), [h, *t]):';
    expectScope(src, 'a', 'variable.parameter.craw');
    expectScope(src, 'b', 'variable.parameter.craw');
    expectScope(src, 'h', 'variable.parameter.craw');
    expectScope(src, 't', 'variable.parameter.craw');
  });

  it('gen, template and macro names', () => {
    expectScope('gen fib():', 'fib', 'entity.name.function.craw');
    expectScope('gen fib():', 'gen', 'storage.type.function.generator.craw');
    expectScope('template log(msg, val):', 'log', 'entity.name.function.craw');
    expectScope('template log(msg, val):', 'msg', 'variable.parameter.craw');
    expectScope('macro m(a):', 'm(', 'entity.name.function.craw');
  });

  it('backtick and operator-section function names', () => {
    expectScope('def `∘`(f, g) = x => f(g(x))', '∘', 'entity.name.function.craw');
    expectScope('def `∘`(f, g) = x => f(g(x))', 'f, g', 'variable.parameter.craw');
    expectScope('def (+)(a, b) = add(a, b)', '+', 'entity.name.function');
  });

  it('a user function named like a builtin is still a definition', () => {
    expectScope('def add(a, b) = a + b', 'add', 'entity.name.function.craw');
  });

  it('data / class / struct / trait names', () => {
    expectScope('data Point(x, y)', 'Point', 'entity.name.type.craw');
    expectScope('data Point(x, y)', 'data', 'storage.type.class.data.craw');
    expectScope('class Dog(name) extends Animal(name) with Show:', 'Dog', 'entity.name.type.craw');
    expectScope('class Dog(name) extends Animal(name) with Show:', 'Animal', 'entity.other.inherited-class.craw');
    expectScope('class Dog(name) extends Animal(name) with Show:', 'with', 'storage.modifier');
    expectScope('struct Vec2:', 'Vec2', 'entity.name.type.craw');
    expectScope('trait Show:', 'Show', 'entity.name.type.craw');
  });

  it('decorator-style macro calls', () => {
    expectScope('@memo(10) fib(n)', 'memo', 'entity.name.function.decorator.craw');
    expectScope('@memo(10) fib(n)', '@', 'punctuation.definition.decorator.craw');
    expectScope('@debug', 'debug', 'entity.name.function.decorator');
  });

  it('@x -> e is a lambda, not a decorator', () => {
    const src = 'f = @x, y -> x + y';
    expectScope(src, '@', 'storage.type.function.lambda.craw');
    expectNoScope(src, 'x', 'entity.name.function.decorator');
  });
});

describe('calls and builtins', () => {
  it('builtin functions from crate::transpiler::BUILTINS', () => {
    const keywords = new Set(lexerKeywords());
    const builtins = transpilerBuiltins().filter((b) => /^\w+$/.test(b) && !keywords.has(b));
    expect(builtins.length).toBeGreaterThan(30);
    for (const b of builtins) {
      expectScope(`x = ${b}(y)`, b, 'support.function.builtin.craw');
    }
  });

  it('builtins used as values (e.g. piped) are still builtins', () => {
    expectScope('xs |> len', 'len', 'support.function.builtin.craw');
  });

  it('user calls are not builtins', () => {
    const src = 'total = compute(xs) + printer(1)';
    expectScope(src, 'compute', 'entity.name.function.call.craw');
    expectNoScope(src, 'compute', 'support.function');
    expectNoScope(src, 'printer', 'support.function');
  });

  it('methods named like builtins are members, not builtins', () => {
    const src = 'xs.map(f).len';
    expectScope(src, 'map', 'entity.name.function.member.craw');
    expectNoScope(src, 'map', 'support.function');
    expectScope(src, 'len', 'variable.other.member.craw');
  });

  it('a keyword after . is still a keyword (it lexes as one)', () => {
    expectScope('x.match(y)', 'match', 'keyword.control');
  });

  it('builtin types in expressions', () => {
    expectScope('s = Set(xs)', 'Set', 'support.type.craw');
    expectScope('t: Tuple = x', 'Tuple', 'support.type.craw');
  });
});

describe('rust passthrough', () => {
  it('rust: block body is embedded rust and ends at dedent', () => {
    const src = [
      'def main():',
      '    cores = rust:',
      '        let t = std::thread::available_parallelism();',
      '',
      '        t as i64',
      '    print(cores)',
    ].join('\n');
    expectScope(src, 'rust', 'keyword.other.passthrough.craw');
    expectNoScope(src, 'rust', 'meta.embedded.block.rust');
    expectScope(src, 'cores', 'variable.other.craw');
    expectScope(src, 'let t', 'meta.embedded.block.rust');
    expectScope(src, 't as', 'meta.embedded.block.rust');
    expectNoScope(src, 'print', 'meta.embedded.block.rust');
    expectScope(src, 'print', 'support.function.builtin.craw');
  });

  it('rust: at column 0 ends at the next unindented line', () => {
    const src = 'rust:\n    line 1\n    line 2\nnext_craw_line';
    expectScope(src, 'line 1', 'meta.embedded.block.rust');
    expectScope(src, 'line 2', 'meta.embedded.block.rust');
    expectNoScope(src, 'next_craw_line', 'meta.embedded.block.rust');
    expectScope(src, 'next_craw_line', 'variable.other.craw');
  });

  it('an open Rust string inside a rust: block cannot leak past the dedent', () => {
    const src = 'x = rust:\n    "unterminated\ny = 1';
    expectNoScope(src, 'y', 'meta.embedded');
    expectNoScope(src, 'y', 'string');
  });

  it('code after rust: on the same line is embedded rust (the block\'s first line)', () => {
    expectScope('rust: panic!("x")', 'panic', 'meta.embedded.block.rust');
    expectNoScope('rust: panic!("x")', 'panic', 'invalid.illegal');
    expectScope('rust: #[inline] fn a() {}', 'inline', 'meta.embedded.block.rust');
    expectScope('rust:  # note', 'note', 'comment.line.number-sign.craw');
  });

  it('"rust:" inside a string or comment is not a block', () => {
    const src = 'print("rust:")\n    y = 1';
    expectNoScope(src, 'y', 'meta.embedded');
    const c = '# rust:\n    y = 1';
    expectNoScope(c, 'y', 'meta.embedded');
    expectNoScope('trust: 1\n    y', 'y', 'meta.embedded');
  });

  it('top-level fn items are embedded rust up to the matching brace', () => {
    const src = [
      'fn poll_input() -> String {',
      '    if ready() {',
      '        return format!("{:?}", "}");',
      '    }',
      '    "None".to_string()',
      '}',
      'def after():',
    ].join('\n');
    expectScope(src, 'fn', 'meta.embedded.block.rust');
    expectScope(src, 'fn', 'keyword.other.fn.rust');
    expectNoScope(src, 'fn', 'keyword.control');
    expectScope(src, 'poll_input', 'entity.name.function.rust');
    expectScope(src, 'return', 'meta.embedded.block.rust');
    expectScope(src, 'None', 'meta.embedded.block.rust');
    expectNoScope(src, 'None', 'constant.language.craw');
    expectNoScope(src, 'def', 'meta.embedded.block.rust');
    expectScope(src, 'after', 'entity.name.function.craw');
  });

  it('fn in the middle of a line is an ordinary identifier', () => {
    const src = 'x = fn + 1';
    expectNoScope(src, 'fn', 'meta.embedded');
    expectNoScope(src, 'fn', 'keyword');
  });

  it('use / extern lines at any indent, let only at column 0', () => {
    expectScope('use std::thread;', 'std', 'meta.embedded.block.rust');
    expectScope('    use std::io;', 'std', 'meta.embedded.block.rust');
    expectScope('extern crate rand;', 'rand', 'meta.embedded.block.rust');
    expectScope('let x = 5;', 'x', 'meta.embedded.block.rust');
    expectNoScope('    let x = 5', 'x', 'meta.embedded');
    expectNoScope('use std::thread\nx = 1', 'x', 'meta.embedded');
  });

  it('inline ®(...), ⚙(...), 🦀(...) and \\(...) escapes', () => {
    for (const trig of ['®', '⚙', '🦀', '\\']) {
      const src = `x = ${trig}(foo(")") + (1)) + y`;
      expectScope(src, trig, 'keyword.other.passthrough.craw');
      expectScope(src, 'foo', 'meta.embedded.block.rust');
      expectScope(src, '+', 'meta.embedded.block.rust');
      expectNoScope(src, 'y', 'meta.embedded');
      expectScope(src, 'y', 'variable.other.craw');
    }
  });

  it('line escapes: trigger + space makes the rest of the line rust', () => {
    const src = '🦀 println!("hi")\nx = 1';
    expectScope(src, 'println', 'meta.embedded.block.rust');
    expectNoScope(src, 'x', 'meta.embedded');
    expectScope('\\ let v = vec![1];', 'vec', 'meta.embedded.block.rust');
  });
});

describe('real-world files', () => {
  function findCraw(dir: string): string[] {
    if (!fs.existsSync(dir)) return [];
    const out: string[] = [];
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        if (entry.name !== 'target' && entry.name !== 'node_modules') out.push(...findCraw(p));
      } else if (entry.name.endsWith('.craw')) {
        out.push(p);
      }
    }
    return out;
  }

  const files = ['samples', 'examples', 'tests'].flatMap((d) => findCraw(path.join(repoRoot, d)));

  it('found sample files', () => {
    expect(files.length).toBeGreaterThan(5);
  });

  it('tokenizes every .craw file under samples/ examples/ tests/ quickly', () => {
    for (const file of files) {
      const text = fs.readFileSync(file, 'utf8').replace(/\r\n/g, '\n');
      const start = Date.now();
      let state = vsctm.INITIAL;
      for (const line of text.split('\n')) {
        const r = grammar.tokenizeLine(line, state, 1000);
        expect(r.stoppedEarly, `${file}: ${line}`).toBe(false);
        state = r.ruleStack;
      }
      expect(Date.now() - start, file).toBeLessThan(2000);
    }
  });

  it('master_showcase highlights as expected', () => {
    const text = fs.readFileSync(path.join(repoRoot, 'samples', 'master_showcase.craw'), 'utf8').replace(/\r\n/g, '\n');
    expectScope(text, 'thread', 'meta.embedded.block.rust');
    expectScope(text, 'fib', 'entity.name.function.craw');
    expectScope(text, 'available_parallelism', 'meta.embedded.block.rust');
    expectNoScope(text, 'print("Available Cores', 'meta.embedded');
  });

  it('pathological input does not backtrack catastrophically', () => {
    const lines = [
      '('.repeat(5000),
      'a'.repeat(20000) + ' rust',
      '@' + 'a, '.repeat(3000) + 'b',
      '"' + 'x\\'.repeat(5000),
      'x = ' + '"a" '.repeat(3000) + 'rust',
      '(' + '+'.repeat(10000),
      '1'.repeat(10000) + '.' + '_'.repeat(5000),
      'def f(' + 'a: List['.repeat(2000),
      'f"' + '{'.repeat(3000),
      '.'.repeat(10000),
    ];
    const start = Date.now();
    for (const line of lines) {
      const r = grammar.tokenizeLine(line, vsctm.INITIAL, 2000);
      expect(r.stoppedEarly).toBe(false);
    }
    expect(Date.now() - start).toBeLessThan(3000);
  });
});

describe('Python-style imports', () => {
  it('scopes every import form', () => {
    expectScope('import shapes.circle as c', 'import', 'keyword.control.import');
    expectScope('import shapes.circle as c', 'as', 'keyword.control.import');
    expectScope('import a, b', 'b', 'variable', 0);
    expectScope('from ..pkg.mod import x', 'from', 'keyword.control.import');
    expectScope('from ..pkg.mod import x', 'import', 'keyword.control.import');
    expectScope('from . import helpers', 'import', 'keyword.control.import');
    expectScope('from geometry import *', 'from', 'keyword.control.import');
  });

  it('keeps parenthesized import lists highlighted across lines', () => {
    const src = 'from geometry import (\n    area as a,\n    Point,\n)\nx = 1';
    expectScope(src, 'as', 'keyword.control.import');
    expectScope(src, '1', 'constant.numeric');
  });

  it('highlights a qualified pattern', () => {
    expectScope('match p:\n    case geometry.Point(x, y):\n        pass', 'case', 'keyword.control');
  });
});
