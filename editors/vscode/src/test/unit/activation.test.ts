import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as vscode from 'vscode';
import * as ext from '../../extension';

/**
 * Activates the real extension against the mocked `vscode` API (aliased
 * in vitest.config.ts) and a fake language client, then drives its
 * commands the way a user would.
 */

const clients: any[] = [];

vi.mock('vscode-languageclient/node', () => {
  class LanguageClient {
    state = 1;
    running = false;
    trace: unknown;
    requests: { method: string; params: unknown }[] = [];
    respond: (method: string, params: any) => unknown = () => null;
    initializeResult = { serverInfo: { name: 'craw-lsp', version: '9.9.9' } };
    private stateListeners: ((e: any) => void)[] = [];
    constructor(
      readonly id: string,
      readonly name: string,
      readonly serverOptions: any,
      readonly clientOptions: any,
    ) {
      clients.push(this);
    }
    setTrace(t: unknown) {
      this.trace = t;
    }
    onDidChangeState(l: (e: any) => void) {
      this.stateListeners.push(l);
      return { dispose() {} };
    }
    async start() {
      this.running = true;
      this.state = 2;
      for (const l of this.stateListeners) l({ oldState: 3, newState: 2 });
    }
    async stop() {
      this.running = false;
      this.state = 1;
      for (const l of this.stateListeners) l({ oldState: 2, newState: 1 });
    }
    isRunning() {
      return this.running;
    }
    needsStop() {
      return this.running;
    }
    async sendRequest(method: string, params: unknown) {
      this.requests.push({ method, params });
      return this.respond(method, params);
    }
  }
  return {
    LanguageClient,
    RevealOutputChannelOn: { Never: 4 },
    State: { Stopped: 1, Running: 2, Starting: 3 },
    Trace: { Off: 0, Messages: 1, Verbose: 2 },
    TransportKind: { stdio: 0 },
  };
});

const pkg = JSON.parse(fs.readFileSync(path.join(__dirname, '..', '..', '..', 'package.json'), 'utf8'));

let extRoot: string;

function context() {
  return { extensionPath: extRoot, subscriptions: [] as { dispose(): void }[] } as any;
}

function withBundledServer(): void {
  fs.mkdirSync(path.join(extRoot, 'bin'), { recursive: true });
  const name = process.platform === 'win32' ? 'craw-lsp.exe' : 'craw-lsp';
  fs.writeFileSync(path.join(extRoot, 'bin', name), '');
}

async function activate() {
  const ctx = context();
  await ext.activate(ctx);
  return ctx;
}

function openCraw(p: string, opts: Partial<vscode.MockDocument> = {}) {
  const uri = p.startsWith('untitled:') ? new vscode.Uri('untitled', p.slice(9)) : vscode.Uri.file(p);
  const doc = vscode.makeDocument(uri, 'print(1)\n', opts);
  vscode.state.textDocuments.push(doc);
  vscode.state.activeTextEditor = { document: doc };
  return doc;
}

async function run(id: string, ...args: unknown[]) {
  const handler = vscode.state.commands.get(id);
  if (!handler) throw new Error(`${id} not registered`);
  return handler(...args);
}

beforeEach(() => {
  vscode.reset();
  clients.length = 0;
  extRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'craw-ext-'));
});

afterEach(async () => {
  await ext.deactivate();
  fs.rmSync(extRoot, { recursive: true, force: true });
});

describe('activation', () => {
  it('registers exactly the commands package.json contributes', async () => {
    withBundledServer();
    await activate();
    const contributed = pkg.contributes.commands.map((c: any) => c.command).sort();
    expect([...vscode.state.commands.keys()].sort()).toEqual(contributed);
  });

  it('starts the client for saved and untitled Craw documents with plain-object settings', async () => {
    withBundledServer();
    vscode.state.config.set('craw.diagnostics.unused', false);
    await activate();
    expect(clients).toHaveLength(1);
    const opts = clients[0].clientOptions;
    expect(opts.documentSelector).toEqual([
      { scheme: 'file', language: 'craw' },
      { scheme: 'untitled', language: 'craw' },
    ]);
    expect(opts.initializationOptions.diagnostics).toEqual({ unresolved: true, unused: false });
    expect(opts.initializationOptions.inlayHints.parameterNames.enable).toBe(true);
    expect(JSON.parse(JSON.stringify(opts.initializationOptions))).toEqual(opts.initializationOptions);
    expect(clients[0].serverOptions.run.command).toBe(path.join(extRoot, 'bin', process.platform === 'win32' ? 'craw-lsp.exe' : 'craw-lsp'));
  });

  it('reports a missing server with a link to the setting', async () => {
    const oldPath = process.env.PATH;
    process.env.PATH = '';
    try {
      vscode.state.messageAnswers.push('Open Settings');
      await activate();
    } finally {
      process.env.PATH = oldPath;
    }
    expect(clients).toHaveLength(0);
    const err = vscode.state.messages.find((m) => m.level === 'error')!;
    expect(err.text).toContain('not found');
    expect(vscode.state.executed).toContainEqual({
      command: 'workbench.action.openSettings',
      args: ['craw.server.path'],
    });
  });

  it('expands variables in craw.server.path', async () => {
    const custom = path.join(extRoot, 'custom-lsp');
    fs.writeFileSync(custom, '');
    vscode.state.workspaceFolders = [{ uri: vscode.Uri.file(extRoot) }];
    vscode.state.config.set('craw.server.path', '${workspaceFolder}/custom-lsp');
    await activate();
    expect(clients[0].serverOptions.run.command).toBe(path.join(extRoot, 'custom-lsp'));
  });

  it('does not report a crash when the user stops the server', async () => {
    withBundledServer();
    await activate();
    await run('craw.stopServer');
    expect(vscode.state.messages.filter((m) => m.text.includes('stopped unexpectedly'))).toEqual([]);
  });

  it('start is idempotent while the server runs', async () => {
    withBundledServer();
    await activate();
    await run('craw.startServer');
    expect(clients).toHaveLength(1);
    await run('craw.restartServer');
    expect(clients).toHaveLength(2);
  });

  it('prompts to restart when a server setting changes', async () => {
    withBundledServer();
    await activate();
    vscode.state.messageAnswers.push('Restart Server');
    for (const l of vscode.state.configListeners) {
      await l({ affectsConfiguration: (s: string) => 'craw.server.path'.startsWith(s) });
    }
    expect(clients).toHaveLength(2);
  });
});

describe('run commands', () => {
  beforeEach(() => withBundledServer());

  it('saves a dirty file, then runs it from its own directory', async () => {
    await activate();
    const doc = openCraw('/w/src/main.craw', { isDirty: true });
    await run('craw.run');
    expect(doc.saved).toBe(1);
    const task = vscode.state.tasks[0];
    expect([task.execution.command, ...task.execution.args]).toEqual(['craw', 'run', '/w/src/main.craw']);
    expect(task.execution.options?.cwd).toBe('/w/src');
    expect(task.problemMatchers).toBe('$craw');
  });

  it('respects craw.run.saveBeforeRun = false', async () => {
    vscode.state.config.set('craw.run.saveBeforeRun', false);
    await activate();
    const doc = openCraw('/w/main.craw', { isDirty: true });
    await run('craw.run');
    expect(doc.saved).toBe(0);
    expect(vscode.state.tasks).toHaveLength(1);
  });

  it('asks to save an untitled file instead of running a bogus path', async () => {
    await activate();
    openCraw('untitled:Untitled-1');
    vscode.state.messageAnswers.push(undefined);
    await run('craw.run');
    expect(vscode.state.tasks).toHaveLength(0);
    expect(vscode.state.messages.some((m) => m.level === 'warning' && m.text.includes('Save'))).toBe(true);
  });

  it('builds with the rustc problem matcher and runs release with --release', async () => {
    await activate();
    openCraw('/w/main.craw');
    await run('craw.build');
    await run('craw.runRelease');
    expect(vscode.state.tasks[0].problemMatchers).toBe('$craw-rustc');
    expect(vscode.state.tasks[1].execution.args).toEqual(['run', '/w/main.craw', '--release']);
  });

  it('runs a file passed from the explorer context menu', async () => {
    await activate();
    await run('craw.transpile', vscode.Uri.file('/w/other.craw'));
    expect(vscode.state.tasks[0].execution.args).toEqual(['transpile', '/w/other.craw']);
  });

  it('errors when there is no Craw file', async () => {
    await activate();
    await run('craw.run');
    expect(vscode.state.tasks).toHaveLength(0);
    expect(vscode.state.messages.some((m) => m.level === 'error')).toBe(true);
  });

  it('opens the REPL in an interactive terminal using the configured interpreter', async () => {
    vscode.state.config.set('craw.interpreter.path', '/opt/crawler');
    await activate();
    await run('craw.openRepl');
    expect(vscode.state.tasks).toHaveLength(0);
    expect(vscode.state.terminals[0].shellPath).toBe('/opt/crawler');
    expect(vscode.state.terminals[0].shown).toBe(true);
  });

  it('provides tasks for the active file and resolves ${workspaceFolder} in task files', async () => {
    vscode.state.workspaceFolders = [{ uri: vscode.Uri.file('/ws') }];
    await activate();
    const provider = vscode.state.taskProviders.get('craw');
    expect(provider.provideTasks()).toEqual([]);
    openCraw('/ws/a.craw');
    expect(provider.provideTasks().map((t: any) => t.definition.command)).toEqual(['run', 'build', 'transpile']);
    const resolved = provider.resolveTask({ definition: { type: 'craw', command: 'build', file: '${workspaceFolder}/b.craw' } });
    expect(resolved.execution.args).toEqual(['build', '/ws/b.craw']);
    expect(resolved.group).toBe('build');
    expect(provider.resolveTask({ definition: { type: 'craw', command: 'check' } })).toBeUndefined();
  });
});

describe('views and lenses', () => {
  beforeEach(() => withBundledServer());

  it('shows generated Rust from the server beside the editor', async () => {
    await activate();
    const doc = openCraw('/w/main.craw');
    clients[0].respond = (method: string) => (method === 'craw/transpile' ? { rust: 'fn main() {}' } : null);
    await run('craw.showTranspiledRust');
    const shown = vscode.state.shown[0];
    expect(shown.document.uri.path).toBe('/main.craw.rs');
    expect(shown.options.viewColumn).toBe(vscode.ViewColumn.Beside);
    const provider = vscode.state.contentProviders.get('craw-view');
    const content = await provider.provideTextDocumentContent(shown.document.uri, {});
    expect(content).toBe('fn main() {}');
    expect(clients[0].requests[0]).toEqual({
      method: 'craw/transpile',
      params: { textDocument: { uri: doc.uri.toString() } },
    });
  });

  it('renders server errors as comments in the view', async () => {
    await activate();
    openCraw('/w/main.craw');
    clients[0].respond = () => {
      throw new Error('1:5: unexpected token\n2:1: another');
    };
    await run('craw.showTranspiledRust');
    const provider = vscode.state.contentProviders.get('craw-view');
    const content = await provider.provideTextDocumentContent(vscode.state.shown[0].document.uri, {});
    expect(content).toBe('// 1:5: unexpected token\n// 2:1: another');
  });

  it('shows the syntax tree via craw/syntaxTree', async () => {
    await activate();
    openCraw('/w/main.craw');
    clients[0].respond = () => '[Assign(...)]';
    await run('craw.showSyntaxTree');
    const provider = vscode.state.contentProviders.get('craw-view');
    const content = await provider.provideTextDocumentContent(vscode.state.shown[0].document.uri, {});
    expect(content).toBe('[Assign(...)]');
    expect(clients[0].requests[0].method).toBe('craw/syntaxTree');
  });

  it('refreshes open views when their source changes', async () => {
    vi.useFakeTimers();
    try {
      await activate();
      const doc = openCraw('/w/main.craw');
      await run('craw.showTranspiledRust');
      const provider = vscode.state.contentProviders.get('craw-view');
      const fired: unknown[] = [];
      provider.onDidChange((u: unknown) => fired.push(u));
      for (const l of vscode.state.changeListeners) l({ document: doc });
      for (const l of vscode.state.changeListeners) l({ document: doc });
      expect(fired).toHaveLength(0);
      vi.advanceTimersByTime(400);
      expect(fired).toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it('offers run lenses, which can be turned off', async () => {
    await activate();
    const { provider, selector } = vscode.state.codeLensProviders[0];
    expect(selector).toEqual(ext.DOCUMENT_SELECTOR);
    const doc = vscode.makeDocument(vscode.Uri.file('/w/a.craw'), '# hi\nprint(1)\n');
    const lenses = provider.provideCodeLenses(doc);
    expect(lenses.map((l: any) => l.command.command)).toEqual(['craw.run', 'craw.build', 'craw.showTranspiledRust']);
    expect(lenses[0].range.startLine).toBe(1);
    expect(lenses[0].command.arguments).toEqual([doc.uri]);
    vscode.state.config.set('craw.lens.run.enable', false);
    expect(provider.provideCodeLenses(doc)).toEqual([]);
  });
});
