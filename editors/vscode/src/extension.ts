import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
  CancellationToken,
  CodeLens,
  CodeLensProvider,
  commands,
  ConfigurationChangeEvent,
  Event,
  EventEmitter,
  ExtensionContext,
  languages,
  OutputChannel,
  Range,
  ShellExecution,
  StatusBarAlignment,
  StatusBarItem,
  Task,
  TaskDefinition,
  TaskGroup,
  TaskProvider,
  TaskScope,
  tasks,
  TextDocument,
  TextDocumentContentProvider,
  TextEditor,
  Uri,
  ViewColumn,
  window,
  workspace,
} from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  RevealOutputChannelOn,
  ServerOptions,
  State,
  Trace,
  TransportKind,
} from 'vscode-languageclient/node';
import { runLenses } from './codeLens';
import { buildCommandLine, CommandKind } from './commands';
import { buildServerSettings, resolveConfiguredPath, VariableContext } from './config';
import { resolveServerPath, ResolveResult } from './serverPath';
import { ServerStatus, statusFor } from './status';
import { decodeView, encodeView, VIEW_SCHEME, ViewKind } from './virtualDocs';

let client: LanguageClient | undefined;
// Set while we stop the client ourselves, so the Running -> Stopped
// transition isn't reported as a crash.
let stopping = false;
let outputChannel: OutputChannel | undefined;
let statusBarItem: StatusBarItem | undefined;
let extensionContext: ExtensionContext | undefined;

/** Documents the language server handles: saved files and unsaved buffers. */
export const DOCUMENT_SELECTOR = [
  { scheme: 'file', language: 'craw' },
  { scheme: 'untitled', language: 'craw' },
];

function binaryName(): string {
  return process.platform === 'win32' ? 'craw-lsp.exe' : 'craw-lsp';
}

function findOnPath(name: string): string | undefined {
  const pathEnv = process.env.PATH ?? '';
  for (const dir of pathEnv.split(path.delimiter)) {
    if (!dir) continue;
    const candidate = path.join(dir, name);
    if (fs.existsSync(candidate)) return candidate;
  }
  return undefined;
}

function variableContext(): VariableContext {
  return {
    workspaceFolder: workspace.workspaceFolders?.[0]?.uri.fsPath,
    userHome: os.homedir(),
    env: process.env,
  };
}

function workspaceBuilds(): string[] {
  const useWorkspaceBuild = workspace.getConfiguration('craw').get<boolean>('server.useWorkspaceBuild', false);
  if (!useWorkspaceBuild) return [];
  const folder = workspace.workspaceFolders?.[0];
  if (!folder) return [];
  return ['release', 'debug'].map((profile) => path.join(folder.uri.fsPath, 'target', profile, binaryName()));
}

async function reportMissingServer(result: { kind: 'missing'; tried: string[] }): Promise<void> {
  const action = await window.showErrorMessage(
    `Craw language server not found. Tried:\n${result.tried.join('\n')}`,
    'Open Settings',
  );
  if (action === 'Open Settings') {
    await commands.executeCommand('workbench.action.openSettings', 'craw.server.path');
  }
}

function traceFromSetting(value: string): Trace {
  switch (value) {
    case 'messages':
      return Trace.Messages;
    case 'verbose':
      return Trace.Verbose;
    default:
      return Trace.Off;
  }
}

function serverVersion(): string | undefined {
  return client?.initializeResult?.serverInfo?.version;
}

function setStatus(status: ServerStatus): void {
  if (!statusBarItem) return;
  const { text, tooltip } = statusFor(status, serverVersion());
  statusBarItem.text = text;
  statusBarItem.tooltip = tooltip;
  statusBarItem.show();
}

function serverSettings() {
  const config = workspace.getConfiguration('craw');
  return buildServerSettings((key, fallback) => config.get(key, fallback));
}

async function startClient(context: ExtensionContext): Promise<void> {
  // Already running, or a start is in flight: nothing to do.
  if (client?.isRunning() || client?.state === State.Starting) return;
  // A client left over from a failed start: dispose of it first.
  if (client) await stopClient();
  const config = workspace.getConfiguration('craw');
  const vars = variableContext();
  const setting = config.get<string>('server.path');
  const result: ResolveResult = resolveServerPath({
    setting: setting ? resolveConfiguredPath(setting, vars) : undefined,
    env: process.env.CRAW_LSP_PATH,
    extensionRoot: context.extensionPath,
    platform: process.platform,
    exists: (p) => fs.existsSync(p),
    workspaceBuilds: workspaceBuilds(),
    onPath: () => findOnPath(binaryName()),
  });

  if (result.kind === 'missing') {
    setStatus('stopped');
    await reportMissingServer(result);
    return;
  }

  const command = result.path;
  const extraEnv = config.get<Record<string, string>>('server.extraEnv', {});
  outputChannel?.appendLine(`Starting ${command} (${result.source})`);

  const serverOptions: ServerOptions = {
    run: { command, transport: TransportKind.stdio, options: { env: { ...process.env, ...extraEnv } } },
    debug: { command, transport: TransportKind.stdio, options: { env: { ...process.env, ...extraEnv } } },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: DOCUMENT_SELECTOR,
    outputChannel,
    revealOutputChannelOn: RevealOutputChannelOn.Never,
    synchronize: { configurationSection: 'craw' },
    initializationOptions: serverSettings(),
  };

  client = new LanguageClient('crawLanguageServer', 'Craw Language Server', serverOptions, clientOptions);
  client.setTrace(traceFromSetting(config.get<string>('trace.server', 'off')));

  client.onDidChangeState((e) => {
    if (e.newState === State.Starting) setStatus('starting');
    else if (e.newState === State.Running) setStatus('running');
    else if (e.newState === State.Stopped) {
      setStatus('stopped');
      if (e.oldState === State.Running && !stopping) {
        window.showErrorMessage('The Craw language server stopped unexpectedly.', 'Show Output').then((action) => {
          if (action === 'Show Output') outputChannel?.show();
        });
      }
    }
  });

  setStatus('starting');
  await client.start();
  // The version is only known once `initialize` has answered.
  setStatus('running');
}

async function stopClient(): Promise<void> {
  if (!client) return;
  const current = client;
  client = undefined;
  stopping = true;
  try {
    if (current.needsStop()) await current.stop();
  } finally {
    stopping = false;
  }
  setStatus('stopped');
}

async function restartClient(): Promise<void> {
  await stopClient();
  if (extensionContext) await startClient(extensionContext);
}

async function showServerMenu(): Promise<void> {
  const pick = await window.showQuickPick(
    [
      { label: 'Restart Server', action: 'restart' },
      { label: 'Stop Server', action: 'stop' },
      { label: 'Start Server', action: 'start' },
      { label: 'Show Server Output', action: 'output' },
    ],
    { placeHolder: `Craw Language Server${serverVersion() ? ` ${serverVersion()}` : ''}` },
  );
  if (!pick) return;
  if (pick.action === 'restart') await restartClient();
  else if (pick.action === 'stop') await stopClient();
  else if (pick.action === 'start') {
    if (extensionContext) await startClient(extensionContext);
  } else if (pick.action === 'output') outputChannel?.show();
}

function cliPath(): string {
  return resolveConfiguredPath(workspace.getConfiguration('craw').get<string>('cli.path', 'craw') || 'craw', variableContext());
}

function interpreterPath(): string {
  return resolveConfiguredPath(
    workspace.getConfiguration('craw').get<string>('interpreter.path', 'crawler') || 'crawler',
    variableContext(),
  );
}

function crawDocument(uri?: Uri): TextDocument | undefined {
  if (uri) return workspace.textDocuments.find((d) => d.uri.toString() === uri.toString());
  const editor: TextEditor | undefined = window.activeTextEditor;
  if (editor?.document.languageId === 'craw') return editor.document;
  return undefined;
}

/**
 * The on-disk file to hand the CLI. The CLI reads the file, not the
 * editor buffer, so a dirty document is saved first (unless
 * `craw.run.saveBeforeRun` is off) and an untitled one must be saved
 * before it can run at all.
 */
async function crawFileForCli(uri?: Uri): Promise<string | undefined> {
  const doc = crawDocument(uri);
  if (!doc) {
    if (uri?.scheme === 'file') return uri.fsPath;
    window.showErrorMessage('No Craw file to run. Open a .craw file first.');
    return undefined;
  }
  if (doc.isUntitled) {
    const action = await window.showWarningMessage('Save this Craw file before running it.', 'Save As…');
    if (action !== 'Save As…') return undefined;
    const saved = await doc.save();
    // Saving an untitled document opens it under its new file URI.
    const active = window.activeTextEditor?.document;
    return saved && active && !active.isUntitled ? active.uri.fsPath : undefined;
  }
  if (doc.isDirty && workspace.getConfiguration('craw').get<boolean>('run.saveBeforeRun', true)) {
    await doc.save();
  }
  return doc.uri.fsPath;
}

async function runTask(name: string, argv: string[], problemMatcher?: string, cwd?: string): Promise<void> {
  const task = new Task(
    { type: 'craw', command: name },
    TaskScope.Workspace,
    `craw ${name}`,
    'craw',
    new ShellExecution(argv[0], argv.slice(1), cwd ? { cwd } : undefined),
    problemMatcher,
  );
  await tasks.executeTask(task);
}

async function runCrawCommand(
  kind: CommandKind,
  name: string,
  uri?: Uri,
  options: { release?: boolean } = {},
): Promise<void> {
  const target = await crawFileForCli(uri);
  if (!target) return;
  const argv = buildCommandLine(kind, target, {
    cli: cliPath(),
    interpreter: interpreterPath(),
    release: options.release,
  });
  await runTask(name, argv, kind === 'build' ? '$craw-rustc' : '$craw', path.dirname(target));
}

/** The REPL is interactive, so it gets a real terminal, not a task. */
function openRepl(): void {
  const argv = buildCommandLine('repl', '', { interpreter: interpreterPath() });
  const terminal = window.createTerminal({
    name: 'Craw REPL',
    shellPath: argv[0],
    shellArgs: argv.slice(1),
    cwd: workspace.workspaceFolders?.[0]?.uri.fsPath,
  });
  terminal.show();
}

async function newProject(): Promise<void> {
  const name = await window.showInputBox({
    prompt: 'New Craw project name',
    validateInput: (v) => (/^[A-Za-z_][A-Za-z0-9_-]*$/.test(v) ? undefined : 'Use letters, digits, `_` or `-`.'),
  });
  if (!name) return;
  const parents = await window.showOpenDialog({
    canSelectFolders: true,
    canSelectFiles: false,
    canSelectMany: false,
    openLabel: 'Select parent folder',
  });
  const parent = parents?.[0];
  if (!parent) return;
  const argv = buildCommandLine('new', name, { cli: cliPath() });
  const task = new Task(
    { type: 'craw', command: 'new' },
    TaskScope.Workspace,
    `craw new ${name}`,
    'craw',
    new ShellExecution(argv[0], argv.slice(1), { cwd: parent.fsPath }),
  );
  const execution = await tasks.executeTask(task);
  const disposable = tasks.onDidEndTaskProcess((e) => {
    if (e.execution !== execution) return;
    disposable.dispose();
    if (e.exitCode !== 0) return;
    void window
      .showInformationMessage(`Craw project "${name}" created.`, 'Open Folder')
      .then((action) => {
        if (action === 'Open Folder') {
          void commands.executeCommand('vscode.openFolder', Uri.joinPath(parent, name));
        }
      });
  });
}

/**
 * Serves `craw-view:` documents by asking the server, and re-renders a
 * view (debounced) whenever its source document changes, so a syntax-tree
 * or generated-Rust view tracks edits live.
 */
class ViewProvider implements TextDocumentContentProvider {
  private readonly changed = new EventEmitter<Uri>();
  readonly onDidChange: Event<Uri> = this.changed.event;
  private readonly open = new Map<string, Uri>();
  private timer: ReturnType<typeof setTimeout> | undefined;

  track(view: Uri): void {
    this.open.set(view.toString(), view);
  }

  sourceChanged(source: string): void {
    const views = [...this.open.values()].filter((v) => decodeView(v.query)?.source === source);
    if (views.length === 0) return;
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => views.forEach((v) => this.changed.fire(v)), 300);
  }

  forget(view: Uri): void {
    this.open.delete(view.toString());
  }

  async provideTextDocumentContent(uri: Uri, _token: CancellationToken): Promise<string> {
    const view = decodeView(uri.query);
    if (!view) return '// Unknown view.';
    if (!client?.isRunning()) return '// The Craw language server is not running.';
    const params = { textDocument: { uri: view.source } };
    try {
      if (view.kind === 'syntax-tree') {
        return await client.sendRequest<string>('craw/syntaxTree', params);
      }
      const result = await client.sendRequest<{ rust: string }>('craw/transpile', params);
      return result.rust;
    } catch (e: any) {
      const message = String(e?.message ?? e);
      return message
        .split('\n')
        .map((l) => `// ${l}`)
        .join('\n');
    }
  }
}

// Created per activation, so no view state outlives the extension.
let views: ViewProvider | undefined;

async function showView(kind: ViewKind): Promise<void> {
  const doc = crawDocument();
  if (!doc) {
    window.showErrorMessage('Open a Craw file first.');
    return;
  }
  const parts = encodeView(kind, doc.uri.toString());
  const uri = Uri.from(parts);
  views?.track(uri);
  const viewDoc = await workspace.openTextDocument(uri);
  await window.showTextDocument(viewDoc, { viewColumn: ViewColumn.Beside, preview: true, preserveFocus: true });
}

class RunLensProvider implements CodeLensProvider {
  private readonly changed = new EventEmitter<void>();
  readonly onDidChangeCodeLenses = this.changed.event;

  refresh(): void {
    this.changed.fire();
  }

  provideCodeLenses(document: TextDocument): CodeLens[] {
    if (!workspace.getConfiguration('craw').get<boolean>('lens.run.enable', true)) return [];
    return runLenses(document.getText()).map(
      (l) =>
        new CodeLens(new Range(l.line, 0, l.line, 0), {
          title: l.title,
          command: l.command,
          arguments: [document.uri],
        }),
    );
  }
}

interface CrawTaskDefinition extends TaskDefinition {
  command: 'build' | 'run' | 'transpile';
  file?: string;
  release?: boolean;
}

function activeCrawFile(): string | undefined {
  const doc = crawDocument();
  return doc && !doc.isUntitled ? doc.uri.fsPath : undefined;
}

function crawTaskArgv(def: CrawTaskDefinition): string[] | undefined {
  const cli = cliPath();
  // Reject commands this extension doesn't know (e.g. a `check` task left
  // in an old tasks.json) rather than falling through to `transpile`.
  if (!['build', 'run', 'transpile'].includes(def.command)) return undefined;
  const target = def.file ? resolveConfiguredPath(def.file, variableContext()) : activeCrawFile();
  if (!target) return undefined;
  const kind: CommandKind = def.command === 'run' ? 'run' : def.command === 'build' ? 'build' : 'transpile';
  return buildCommandLine(kind, target, { cli, release: def.release });
}

function makeCrawTask(def: CrawTaskDefinition): Task | undefined {
  const argv = crawTaskArgv(def);
  if (!argv) return undefined;
  const task = new Task(
    def,
    TaskScope.Workspace,
    `craw ${def.command}`,
    'craw',
    new ShellExecution(argv[0], argv.slice(1)),
    def.command === 'build' ? '$craw-rustc' : '$craw',
  );
  if (def.command === 'build') task.group = TaskGroup.Build;
  return task;
}

class CrawTaskProvider implements TaskProvider {
  provideTasks(): Task[] {
    return (['run', 'build', 'transpile'] as const)
      .map((command) => makeCrawTask({ type: 'craw', command }))
      .filter((t): t is Task => t !== undefined);
  }
  resolveTask(task: Task): Task | undefined {
    const def = task.definition as CrawTaskDefinition;
    if (def.type !== 'craw' || !def.command) return undefined;
    return makeCrawTask(def);
  }
}

/** Every command this extension registers, keyed by id. */
function commandHandlers(context: ExtensionContext): Record<string, (...args: any[]) => unknown> {
  return {
    'craw.restartServer': restartClient,
    'craw.stopServer': stopClient,
    'craw.startServer': () => startClient(context),
    'craw.showServerOutput': () => outputChannel?.show(),
    'craw.showServerMenu': showServerMenu,
    'craw.run': (uri?: Uri) => runCrawCommand('run', 'run', uri),
    'craw.runRelease': (uri?: Uri) => runCrawCommand('run', 'run (release)', uri, { release: true }),
    'craw.build': (uri?: Uri) => runCrawCommand('build', 'build', uri),
    'craw.transpile': (uri?: Uri) => runCrawCommand('transpile', 'transpile', uri),
    'craw.runInterpreter': (uri?: Uri) => runCrawCommand('runInterpreter', 'interpreter', uri),
    'craw.openRepl': openRepl,
    'craw.newProject': newProject,
    'craw.showSyntaxTree': () => showView('syntax-tree'),
    'craw.showTranspiledRust': () => showView('rust'),
  };
}

/** Settings whose change requires restarting the server process. */
export const RESTART_SETTINGS = ['craw.server.path', 'craw.server.extraEnv', 'craw.server.useWorkspaceBuild'];

export async function activate(context: ExtensionContext): Promise<void> {
  extensionContext = context;
  outputChannel = window.createOutputChannel('Craw Language Server');
  statusBarItem = window.createStatusBarItem(StatusBarAlignment.Left, 100);
  statusBarItem.command = 'craw.showServerMenu';
  context.subscriptions.push(outputChannel, statusBarItem);

  for (const [id, handler] of Object.entries(commandHandlers(context))) {
    context.subscriptions.push(commands.registerCommand(id, handler));
  }

  const lenses = new RunLensProvider();
  const viewProvider = new ViewProvider();
  views = viewProvider;
  context.subscriptions.push(
    tasks.registerTaskProvider('craw', new CrawTaskProvider()),
    workspace.registerTextDocumentContentProvider(VIEW_SCHEME, viewProvider),
    languages.registerCodeLensProvider(DOCUMENT_SELECTOR, lenses),
    workspace.onDidChangeTextDocument((e) => viewProvider.sourceChanged(e.document.uri.toString())),
    workspace.onDidCloseTextDocument((d) => {
      if (d.uri.scheme === VIEW_SCHEME) viewProvider.forget(d.uri);
    }),
  );

  context.subscriptions.push(
    workspace.onDidChangeConfiguration(async (e: ConfigurationChangeEvent) => {
      if (e.affectsConfiguration('craw.lens')) lenses.refresh();
      if (RESTART_SETTINGS.some((s) => e.affectsConfiguration(s))) {
        const action = await window.showInformationMessage('Craw server settings changed.', 'Restart Server');
        if (action === 'Restart Server') await restartClient();
      } else if (e.affectsConfiguration('craw.trace.server') && client) {
        client.setTrace(traceFromSetting(workspace.getConfiguration('craw').get<string>('trace.server', 'off')));
      }
    }),
  );

  await startClient(context);
}

export function deactivate(): Thenable<void> | undefined {
  views = undefined;
  return stopClient();
}
