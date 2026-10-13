import * as fs from 'fs';
import * as path from 'path';
import { describe, expect, it } from 'vitest';

const root = path.join(__dirname, '..', '..', '..');

function readJson(relPath: string): any {
  return JSON.parse(fs.readFileSync(path.join(root, relPath), 'utf8'));
}

describe('language-configuration.json', () => {
  const config = readJson('language-configuration.json');

  it('does not declare block comments (Craw has none)', () => {
    expect(config.comments.blockComment).toBeUndefined();
  });

  it('indents after a statement header ending in :', () => {
    const rule = config.onEnterRules.find((r: any) => new RegExp(r.beforeText).test('def f():'));
    expect(rule).toBeDefined();
    expect(rule.action.indentAction).toBe('indent');
  });

  it('outdents after return/break/continue/pass', () => {
    const rule = config.onEnterRules.find((r: any) => new RegExp(r.beforeText).test('    return x'));
    expect(rule).toBeDefined();
    expect(rule.action.indentAction).toBe('outdent');
  });

  it('treats Unicode operators and identifiers as whole words', () => {
    const wordPattern = new RegExp(config.wordPattern, 'g');
    expect('a ≈ b'.match(wordPattern)).toEqual(['a', '≈', 'b']);
    expect('my_var + 1'.match(wordPattern)).toContain('my_var');
  });

  it('declares region folding markers', () => {
    expect(config.folding.markers.start).toBeTruthy();
    expect(config.folding.markers.end).toBeTruthy();
  });
});

describe('snippets/craw.json', () => {
  const snippets = readJson('snippets/craw.json');
  const entries = Object.values(snippets) as { prefix: string; body: string | string[] }[];

  it('every snippet has a tab stop or placeholder', () => {
    for (const s of entries) {
      const body = Array.isArray(s.body) ? s.body.join('\n') : s.body;
      expect(/\$0|\$\{\d+:/.test(body), `snippet ${s.prefix} has no tab stop`).toBe(true);
    }
  });

  it('every snippet prefix is unique', () => {
    const prefixes = entries.map((s) => s.prefix);
    expect(new Set(prefixes).size).toBe(prefixes.length);
  });
});
