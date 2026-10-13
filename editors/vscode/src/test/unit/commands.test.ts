import { describe, expect, it } from 'vitest';
import { buildCommandLine } from '../../commands';

describe('buildCommandLine', () => {
  it('builds a run command, optionally in release mode', () => {
    expect(buildCommandLine('run', '/w/x.craw', { cli: 'craw' })).toEqual(['craw', 'run', '/w/x.craw']);
    expect(buildCommandLine('run', '/w/x.craw', { cli: 'craw', release: true })).toEqual([
      'craw',
      'run',
      '/w/x.craw',
      '--release',
    ]);
  });

  it('builds build and transpile commands', () => {
    expect(buildCommandLine('build', '/w/x.craw', { cli: 'craw' })).toEqual(['craw', 'build', '/w/x.craw']);
    expect(buildCommandLine('transpile', '/w/x.craw', { cli: 'craw' })).toEqual(['craw', 'transpile', '/w/x.craw']);
  });

  it('builds a repl command with no target', () => {
    expect(buildCommandLine('repl')).toEqual(['crawler']);
  });

  it('builds a run-in-interpreter command', () => {
    expect(buildCommandLine('runInterpreter', '/w/x.craw')).toEqual(['crawler', '/w/x.craw']);
  });

  it('builds a new-project command', () => {
    expect(buildCommandLine('new', 'my_app', { cli: 'craw' })).toEqual(['craw', 'new', 'my_app']);
  });

  it('defaults cli to "craw" when not given', () => {
    expect(buildCommandLine('build', 'x.craw')).toEqual(['craw', 'build', 'x.craw']);
  });

  it('keeps a path with spaces as a single argv entry, not shell-joined', () => {
    const argv = buildCommandLine('run', '/w/my file.craw', { cli: 'craw' });
    expect(argv).toEqual(['craw', 'run', '/w/my file.craw']);
    expect(argv.length).toBe(3);
  });

  it('uses a configured interpreter for the REPL and interpreter runs', () => {
    expect(buildCommandLine('repl', '', { interpreter: '/opt/crawler' })).toEqual(['/opt/crawler']);
    expect(buildCommandLine('runInterpreter', 'a.craw', { interpreter: '/opt/crawler' })).toEqual([
      '/opt/crawler',
      'a.craw',
    ]);
  });
});
