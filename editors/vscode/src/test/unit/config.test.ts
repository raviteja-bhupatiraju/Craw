import { describe, expect, it } from 'vitest';
import { buildServerSettings, resolveConfiguredPath, substituteVariables } from '../../config';

const ctx = { workspaceFolder: '/ws', userHome: '/home/u', env: { TOOLS: '/opt/tools' } };

describe('substituteVariables', () => {
  it('expands workspaceFolder, userHome and env variables', () => {
    expect(substituteVariables('${workspaceFolder}/target/craw-lsp', ctx)).toBe('/ws/target/craw-lsp');
    expect(substituteVariables('${userHome}/bin/craw', ctx)).toBe('/home/u/bin/craw');
    expect(substituteVariables('${env:TOOLS}/craw', ctx)).toBe('/opt/tools/craw');
  });

  it('expands a leading tilde only', () => {
    expect(substituteVariables('~/bin/craw-lsp', ctx)).toBe('/home/u/bin/craw-lsp');
    expect(substituteVariables('~', ctx)).toBe('/home/u');
    expect(substituteVariables('/a/~/b', ctx)).toBe('/a/~/b');
  });

  it('blanks unset env variables but keeps unknown variables verbatim', () => {
    expect(substituteVariables('${env:MISSING}craw', ctx)).toBe('craw');
    expect(substituteVariables('${nope}/craw', ctx)).toBe('${nope}/craw');
  });

  it('leaves workspaceFolder alone when there is no workspace', () => {
    expect(substituteVariables('${workspaceFolder}/x', { userHome: '/h' })).toBe('${workspaceFolder}/x');
  });
});

describe('resolveConfiguredPath', () => {
  it('resolves relative paths with a separator against the workspace', () => {
    expect(resolveConfiguredPath('./bin/craw-lsp', ctx)).toBe('/ws/bin/craw-lsp');
    expect(resolveConfiguredPath('target/debug/craw-lsp', ctx)).toBe('/ws/target/debug/craw-lsp');
  });

  it('keeps bare command names for PATH lookup, and absolute paths as-is', () => {
    expect(resolveConfiguredPath('craw', ctx)).toBe('craw');
    expect(resolveConfiguredPath('/usr/bin/craw', ctx)).toBe('/usr/bin/craw');
    expect(resolveConfiguredPath('C:\\tools\\craw.exe', ctx)).toBe('C:\\tools\\craw.exe');
  });

  it('trims whitespace and passes an empty setting through', () => {
    expect(resolveConfiguredPath('  craw  ', ctx)).toBe('craw');
    expect(resolveConfiguredPath('', ctx)).toBe('');
  });
});

describe('buildServerSettings', () => {
  it('defaults everything on, in the nested shape the server parses', () => {
    expect(buildServerSettings((_k, fallback) => fallback)).toEqual({
      diagnostics: { unresolved: true, unused: true },
      inlayHints: {
        parameterNames: { enable: true },
        typeHints: { enable: true },
        returnTypeHints: { enable: true },
      },
    });
  });

  it('reads each value from its dotted setting key', () => {
    const values: Record<string, unknown> = {
      'diagnostics.unused': false,
      'inlayHints.typeHints.enable': false,
    };
    const s = buildServerSettings((k, fallback) => (k in values ? (values[k] as any) : fallback));
    expect(s.diagnostics.unused).toBe(false);
    expect(s.diagnostics.unresolved).toBe(true);
    expect(s.inlayHints.typeHints.enable).toBe(false);
    expect(s.inlayHints.parameterNames.enable).toBe(true);
  });
});
