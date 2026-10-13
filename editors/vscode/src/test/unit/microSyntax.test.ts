import * as fs from 'fs';
import * as yaml from 'js-yaml';
import * as path from 'path';
import { describe, expect, it } from 'vitest';

/**
 * `craw.yaml` (the micro editor syntax at the repo root) must highlight
 * the same keywords the lexer actually has — it drifted once already,
 * advertising `try`/`async`/`pass` and missing `gen`/`while`/`struct`.
 */

const repo = path.join(__dirname, '..', '..', '..', '..', '..');
const syntax: any = yaml.load(fs.readFileSync(path.join(repo, 'craw.yaml'), 'utf8'));
const lexer = fs.readFileSync(path.join(repo, 'src', 'lexer.rs'), 'utf8');

/** Every word the lexer maps to a keyword token (not `Token::Ident`). */
const lexerKeywords = [...lexer.matchAll(/^\s*"([a-z]+)" => Token::(?!Ident)/gm)].map((m) => m[1]);

function statementWords(): string[] {
  const words: string[] = [];
  for (const rule of syntax.rules) {
    const pattern = rule.statement;
    if (typeof pattern !== 'string') continue;
    const m = pattern.match(/\\b\(([^)]*)\)\\b/);
    if (m) words.push(...m[1].split('|'));
  }
  return words;
}

describe('craw.yaml', () => {
  it('reads the keyword table from the lexer', () => {
    expect(lexerKeywords).toEqual(expect.arrayContaining(['def', 'gen', 'while', 'notin']));
  });

  it('highlights every lexer keyword', () => {
    const words = statementWords();
    for (const kw of lexerKeywords) expect(words, kw).toContain(kw);
  });

  it('highlights nothing the lexer does not treat as a keyword', () => {
    for (const w of statementWords()) expect(lexerKeywords, w).toContain(w);
    for (const notKeyword of ['try', 'async', 'await', 'pass', 'let', 'fn', 'elif', 'except']) {
      expect(statementWords()).not.toContain(notKeyword);
    }
  });

  it('treats only the capitalized constants as constants', () => {
    const rule = syntax.rules.find((r: any) => r['constant.bool']);
    expect(rule['constant.bool']).toBe('\\b(True|False|None)\\b');
  });

  it('marks a lone `!` as an error', () => {
    expect(syntax.rules.some((r: any) => r.error === '!')).toBe(true);
  });
});
