import * as fs from 'fs';
import * as path from 'path';
import { describe, expect, it } from 'vitest';
import { crawPattern, rustcPattern } from '../../problemMatchers';

function matchGroups(pattern: string, line: string): string[] | null {
  const m = new RegExp(pattern).exec(line);
  return m ? m.slice(1) : null;
}

describe('crawPattern', () => {
  it('matches a craw compiler diagnostic line', () => {
    const groups = matchGroups(
      crawPattern.regexp,
      'samples/x.craw:12:7: error: expected expression, found end of line',
    );
    expect(groups).not.toBeNull();
    expect(groups![crawPattern.file - 1]).toBe('samples/x.craw');
    expect(groups![crawPattern.line - 1]).toBe('12');
    expect(groups![crawPattern.column - 1]).toBe('7');
    expect(groups![crawPattern.severity - 1]).toBe('error');
    expect(groups![crawPattern.message - 1]).toBe('expected expression, found end of line');
  });
});

describe('rustcPattern', () => {
  it('matches rustc\'s two-line diagnostic form', () => {
    const first = matchGroups(rustcPattern[0].regexp, 'error[E0308]: mismatched types');
    expect(first).not.toBeNull();
    expect(first![rustcPattern[0].severity - 1]).toBe('error');
    expect(first![rustcPattern[0].message - 1]).toBe('mismatched types');

    const second = matchGroups(rustcPattern[1].regexp, '  --> file.rs:10:5');
    expect(second).not.toBeNull();
    expect(second![rustcPattern[1].file - 1]).toBe('file.rs');
    expect(second![rustcPattern[1].line - 1]).toBe('10');
    expect(second![rustcPattern[1].column - 1]).toBe('5');
  });
});

describe('package.json problemMatchers stay in sync with problemMatchers.ts', () => {
  const pkg = JSON.parse(fs.readFileSync(path.join(__dirname, '..', '..', '..', 'package.json'), 'utf8'));
  const matchers: any[] = pkg.contributes.problemMatchers;

  it('$craw matches crawPattern exactly', () => {
    const craw = matchers.find((m) => m.name === 'craw');
    expect(craw.pattern).toEqual(crawPattern);
  });

  it('$craw-rustc matches rustcPattern exactly', () => {
    const rustc = matchers.find((m) => m.name === 'craw-rustc');
    expect(rustc.pattern).toEqual(rustcPattern);
  });
});
