import { describe, expect, it } from 'vitest';
import { resolveServerPath } from '../../serverPath';

const fsWith = (existing: string[]) => (p: string) => existing.includes(p);

describe('resolveServerPath', () => {
  it('prefers the explicit setting', () => {
    const r = resolveServerPath({
      setting: '/custom/craw-lsp',
      env: '/env/craw-lsp',
      extensionRoot: '/ext',
      platform: 'linux',
      exists: fsWith(['/custom/craw-lsp', '/env/craw-lsp', '/ext/bin/craw-lsp']),
    });
    expect(r).toEqual({ kind: 'found', path: '/custom/craw-lsp', source: 'setting' });
  });

  it('falls back to CRAW_LSP_PATH env, then bundled binary', () => {
    expect(
      resolveServerPath({
        setting: undefined,
        env: '/env/craw-lsp',
        extensionRoot: '/ext',
        platform: 'linux',
        exists: fsWith(['/env/craw-lsp']),
      }),
    ).toMatchObject({ path: '/env/craw-lsp', source: 'env' });
    expect(
      resolveServerPath({
        setting: undefined,
        env: undefined,
        extensionRoot: '/ext',
        platform: 'win32',
        exists: fsWith(['/ext/bin/craw-lsp.exe']),
      }),
    ).toMatchObject({ path: '/ext/bin/craw-lsp.exe', source: 'bundled' });
  });

  it('reports a setting that points at a missing file instead of silently falling through', () => {
    expect(
      resolveServerPath({
        setting: '/nope',
        env: undefined,
        extensionRoot: '/ext',
        platform: 'linux',
        exists: fsWith([]),
      }),
    ).toEqual({ kind: 'missing', tried: ['/nope', '/ext/bin/craw-lsp'] });
  });

  it('uses PATH lookup as the last resort', () => {
    expect(
      resolveServerPath({
        setting: undefined,
        env: undefined,
        extensionRoot: '/ext',
        platform: 'linux',
        exists: fsWith([]),
        onPath: () => '/usr/local/bin/craw-lsp',
      }),
    ).toMatchObject({ path: '/usr/local/bin/craw-lsp', source: 'path' });
  });

  it('lists every path tried, including the PATH lookup miss', () => {
    expect(
      resolveServerPath({
        setting: undefined,
        env: undefined,
        extensionRoot: '/ext',
        platform: 'linux',
        exists: fsWith([]),
        onPath: () => undefined,
      }),
    ).toEqual({ kind: 'missing', tried: ['/ext/bin/craw-lsp'] });
  });

  it('prefers a workspace build over the bundled binary when enabled', () => {
    expect(
      resolveServerPath({
        setting: undefined,
        env: undefined,
        extensionRoot: '/ext',
        platform: 'linux',
        workspaceBuilds: ['/ws/target/release/craw-lsp', '/ws/target/debug/craw-lsp'],
        exists: fsWith(['/ws/target/debug/craw-lsp', '/ext/bin/craw-lsp']),
      }),
    ).toEqual({ kind: 'found', path: '/ws/target/debug/craw-lsp', source: 'workspace' });
  });

  it('still lets the explicit setting beat a workspace build', () => {
    expect(
      resolveServerPath({
        setting: '/custom/craw-lsp',
        env: undefined,
        extensionRoot: '/ext',
        platform: 'linux',
        workspaceBuilds: ['/ws/target/release/craw-lsp'],
        exists: fsWith(['/custom/craw-lsp', '/ws/target/release/craw-lsp']),
      }),
    ).toMatchObject({ source: 'setting' });
  });

  it('lists workspace candidates among the paths tried', () => {
    expect(
      resolveServerPath({
        setting: undefined,
        env: undefined,
        extensionRoot: '/ext',
        platform: 'linux',
        workspaceBuilds: ['/ws/target/release/craw-lsp'],
        exists: fsWith([]),
      }),
    ).toEqual({ kind: 'missing', tried: ['/ws/target/release/craw-lsp', '/ext/bin/craw-lsp'] });
  });
});
