import { describe, expect, it } from 'vitest';
import { runLenses } from '../../codeLens';

describe('runLenses', () => {
  it('puts Run / Build / Show Rust on the first line of code', () => {
    const lenses = runLenses('# header\n\n# more\nprint(1)\n');
    expect(lenses.map((l) => l.line)).toEqual([3, 3, 3]);
    expect(lenses.map((l) => l.command)).toEqual(['craw.run', 'craw.build', 'craw.showTranspiledRust']);
    expect(lenses[0].title).toContain('Run');
  });

  it('handles CRLF and code on the first line', () => {
    expect(runLenses('x = 1\r\n')[0].line).toBe(0);
    expect(runLenses('# c\r\nx = 1\r\n')[0].line).toBe(1);
  });

  it('shows nothing for an empty or comment-only file', () => {
    expect(runLenses('')).toEqual([]);
    expect(runLenses('# just a comment\n\n')).toEqual([]);
  });
});
