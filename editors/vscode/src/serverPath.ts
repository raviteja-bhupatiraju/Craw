import * as path from 'path';

export interface ResolveInput {
  setting: string | undefined;
  env: string | undefined;
  extensionRoot: string;
  platform: NodeJS.Platform;
  exists: (p: string) => boolean;
  onPath?: () => string | undefined;
  /**
   * Candidate `target/{release,debug}` binaries from the open workspace,
   * when `craw.server.useWorkspaceBuild` is on. Checked *before* the
   * bundled binary: the flag exists to test a freshly built server, which
   * a (typically older) bundled copy must not shadow.
   */
  workspaceBuilds?: string[];
}

export type ResolveResult =
  | { kind: 'found'; path: string; source: 'setting' | 'env' | 'workspace' | 'bundled' | 'path' }
  | { kind: 'missing'; tried: string[] };

function binaryName(platform: NodeJS.Platform): string {
  return platform === 'win32' ? 'craw-lsp.exe' : 'craw-lsp';
}

function bundledPath(extensionRoot: string, platform: NodeJS.Platform): string {
  const join = extensionRoot.startsWith('/') ? path.posix.join : path.join;
  return join(extensionRoot, 'bin', binaryName(platform));
}

export function resolveServerPath(input: ResolveInput): ResolveResult {
  const { setting, env, extensionRoot, platform, exists, onPath, workspaceBuilds = [] } = input;
  const tried: string[] = [];

  if (setting) {
    tried.push(setting);
    if (exists(setting)) {
      return { kind: 'found', path: setting, source: 'setting' };
    }
  }

  if (env) {
    tried.push(env);
    if (exists(env)) {
      return { kind: 'found', path: env, source: 'env' };
    }
  }

  for (const candidate of workspaceBuilds) {
    tried.push(candidate);
    if (exists(candidate)) {
      return { kind: 'found', path: candidate, source: 'workspace' };
    }
  }

  const bundled = bundledPath(extensionRoot, platform);
  tried.push(bundled);
  if (exists(bundled)) {
    return { kind: 'found', path: bundled, source: 'bundled' };
  }

  if (onPath) {
    const onPathResult = onPath();
    if (onPathResult) {
      return { kind: 'found', path: onPathResult, source: 'path' };
    }
  }

  return { kind: 'missing', tried };
}
