import * as path from 'path';

export interface VariableContext {
  workspaceFolder?: string;
  userHome?: string;
  env?: Record<string, string | undefined>;
}

/**
 * Expands the variables VS Code users expect in path settings:
 * `${workspaceFolder}`, `${userHome}`, `${env:NAME}`, and a leading `~`.
 * Unknown `${...}` variables are left as-is rather than blanked, so a typo
 * shows up verbatim in the "server not found" message.
 */
export function substituteVariables(value: string, ctx: VariableContext): string {
  let out = value;
  if (ctx.userHome && (out === '~' || out.startsWith('~/') || out.startsWith('~\\'))) {
    out = ctx.userHome + out.slice(1);
  }
  return out.replace(/\$\{([^}]+)\}/g, (whole, name: string) => {
    if (name === 'workspaceFolder' && ctx.workspaceFolder) return ctx.workspaceFolder;
    if (name === 'userHome' && ctx.userHome) return ctx.userHome;
    if (name.startsWith('env:')) return ctx.env?.[name.slice(4)] ?? '';
    return whole;
  });
}

/**
 * A path setting as the process should see it: variables expanded, and a
 * relative path containing a directory separator (`./bin/craw-lsp`)
 * resolved against the workspace folder. A bare name (`craw`) is left
 * alone so it's still looked up on `PATH`.
 */
export function resolveConfiguredPath(value: string, ctx: VariableContext): string {
  const expanded = substituteVariables(value.trim(), ctx);
  if (!expanded) return expanded;
  const isAbsolute = path.isAbsolute(expanded) || /^[A-Za-z]:[\\/]/.test(expanded);
  const hasSeparator = expanded.includes('/') || expanded.includes('\\');
  if (!isAbsolute && hasSeparator && ctx.workspaceFolder) {
    return path.join(ctx.workspaceFolder, expanded);
  }
  return expanded;
}

export type SettingGetter = <T>(key: string, fallback: T) => T;

/**
 * The server-relevant subset of the `craw.*` settings, shaped exactly as
 * `parse_server_config` in `craw-lsp` reads it (nested by the dotted
 * setting names). Built explicitly, rather than passing the
 * `WorkspaceConfiguration` proxy, so what the server receives is a plain,
 * testable object.
 */
export function buildServerSettings(get: SettingGetter) {
  return {
    diagnostics: {
      unresolved: get('diagnostics.unresolved', true),
      unused: get('diagnostics.unused', true),
    },
    inlayHints: {
      parameterNames: { enable: get('inlayHints.parameterNames.enable', true) },
      typeHints: { enable: get('inlayHints.typeHints.enable', true) },
      returnTypeHints: { enable: get('inlayHints.returnTypeHints.enable', true) },
    },
  };
}
