export type CommandKind = 'run' | 'build' | 'transpile' | 'repl' | 'runInterpreter' | 'new';

export interface CommandLineOptions {
  cli?: string;
  /** The `crawler` interpreter/REPL binary. */
  interpreter?: string;
  release?: boolean;
}

/**
 * Builds an argv array (never a shell-joined string, so paths with spaces
 * need no quoting) for one of the extension's run/build/transpile/REPL/
 * new-project commands.
 */
export function buildCommandLine(kind: CommandKind, target = '', options: CommandLineOptions = {}): string[] {
  const cli = options.cli ?? 'craw';
  const interpreter = options.interpreter ?? 'crawler';
  switch (kind) {
    case 'run': {
      const argv = [cli, 'run', target];
      if (options.release) argv.push('--release');
      return argv;
    }
    case 'build':
      return [cli, 'build', target];
    case 'transpile':
      return [cli, 'transpile', target];
    case 'repl':
      return [interpreter];
    case 'runInterpreter':
      return [interpreter, target];
    case 'new':
      return [cli, 'new', target];
  }
}
