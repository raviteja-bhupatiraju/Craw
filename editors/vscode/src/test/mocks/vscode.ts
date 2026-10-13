/**
 * A minimal in-memory stand-in for the `vscode` API — just enough surface
 * for `extension.ts` to activate and run its commands under vitest. Every
 * interaction is recorded on `state` so tests can assert on it.
 */
import * as path from 'path';

type Listener<T> = (e: T) => unknown;

export class EventEmitter<T> {
  private listeners: Listener<T>[] = [];
  event = (l: Listener<T>) => {
    this.listeners.push(l);
    return { dispose: () => (this.listeners = this.listeners.filter((x) => x !== l)) };
  };
  fire(e: T) {
    for (const l of this.listeners) l(e);
  }
  dispose() {
    this.listeners = [];
  }
}

export class Uri {
  constructor(
    readonly scheme: string,
    readonly path: string,
    readonly query = '',
  ) {}
  get fsPath() {
    return this.path;
  }
  static file(p: string) {
    return new Uri('file', p);
  }
  static from(parts: { scheme: string; path: string; query?: string }) {
    return new Uri(parts.scheme, parts.path, parts.query ?? '');
  }
  static joinPath(base: Uri, ...segments: string[]) {
    return new Uri(base.scheme, path.posix.join(base.path, ...segments), base.query);
  }
  toString() {
    if (this.scheme === 'untitled') return `untitled:${this.path}`;
    return `${this.scheme}://${this.path}${this.query ? `?${this.query}` : ''}`;
  }
}

export class Range {
  constructor(
    readonly startLine: number,
    readonly startCharacter: number,
    readonly endLine: number,
    readonly endCharacter: number,
  ) {}
}

export class CodeLens {
  constructor(
    readonly range: Range,
    readonly command?: { title: string; command: string; arguments?: unknown[] },
  ) {}
}

export class ShellExecution {
  constructor(
    readonly command: string,
    readonly args: string[],
    readonly options?: { cwd?: string },
  ) {}
}

export class Task {
  group: unknown;
  constructor(
    readonly definition: any,
    readonly scope: unknown,
    readonly name: string,
    readonly source: string,
    readonly execution: ShellExecution,
    readonly problemMatchers?: string,
  ) {}
}

export const TaskGroup = { Build: 'build' };
export const TaskScope = { Workspace: 'workspace', Global: 'global' };
export const StatusBarAlignment = { Left: 1, Right: 2 };
export const ViewColumn = { Beside: -2, Active: -1 };

export interface MockDocument {
  uri: Uri;
  languageId: string;
  isUntitled: boolean;
  isDirty: boolean;
  text: string;
  saved: number;
  getText(): string;
  save(): Promise<boolean>;
}

export function makeDocument(uri: Uri, text = '', opts: Partial<MockDocument> = {}): MockDocument {
  const doc: MockDocument = {
    uri,
    languageId: 'craw',
    isUntitled: uri.scheme === 'untitled',
    isDirty: false,
    text,
    saved: 0,
    getText: () => doc.text,
    save: async () => {
      doc.saved++;
      doc.isDirty = false;
      return true;
    },
    ...opts,
  };
  return doc;
}

export const state = {
  commands: new Map<string, (...args: any[]) => unknown>(),
  executed: [] as { command: string; args: unknown[] }[],
  messages: [] as { level: 'error' | 'warning' | 'info'; text: string; items: string[] }[],
  /** Answers popped by the next showXMessage call (undefined = dismissed). */
  messageAnswers: [] as (string | undefined)[],
  tasks: [] as Task[],
  taskProviders: new Map<string, any>(),
  terminals: [] as any[],
  contentProviders: new Map<string, any>(),
  codeLensProviders: [] as { selector: unknown; provider: any }[],
  config: new Map<string, unknown>(),
  configListeners: [] as Listener<any>[],
  changeListeners: [] as Listener<any>[],
  workspaceFolders: undefined as { uri: Uri }[] | undefined,
  textDocuments: [] as MockDocument[],
  activeTextEditor: undefined as { document: MockDocument } | undefined,
  shown: [] as { document: any; options: any }[],
  outputShown: 0,
};

export function reset() {
  state.commands.clear();
  state.executed = [];
  state.messages = [];
  state.messageAnswers = [];
  state.tasks = [];
  state.taskProviders.clear();
  state.terminals = [];
  state.contentProviders.clear();
  state.codeLensProviders = [];
  state.config.clear();
  state.configListeners = [];
  state.changeListeners = [];
  state.workspaceFolders = undefined;
  state.textDocuments = [];
  state.activeTextEditor = undefined;
  state.shown = [];
  state.outputShown = 0;
}

const disposable = { dispose() {} };

function message(level: 'error' | 'warning' | 'info') {
  return async (text: string, ...items: string[]) => {
    state.messages.push({ level, text, items });
    return state.messageAnswers.shift();
  };
}

export const window = {
  createOutputChannel: () => ({
    appendLine() {},
    show() {
      state.outputShown++;
    },
    dispose() {},
  }),
  createStatusBarItem: () => ({ text: '', tooltip: '', command: '', show() {}, hide() {}, dispose() {} }),
  showErrorMessage: message('error'),
  showWarningMessage: message('warning'),
  showInformationMessage: message('info'),
  showQuickPick: async () => undefined,
  showInputBox: async () => undefined,
  showOpenDialog: async () => undefined,
  createTerminal: (opts: any) => {
    const t = { ...opts, shown: false, show() { t.shown = true; }, dispose() {} };
    state.terminals.push(t);
    return t;
  },
  showTextDocument: async (document: any, options: any) => {
    state.shown.push({ document, options });
  },
  get activeTextEditor() {
    return state.activeTextEditor;
  },
};

export const workspace = {
  getConfiguration: (section?: string) => ({
    get: (key: string, fallback?: unknown) => {
      const full = section ? `${section}.${key}` : key;
      return state.config.has(full) ? state.config.get(full) : fallback;
    },
  }),
  get workspaceFolders() {
    return state.workspaceFolders;
  },
  get textDocuments() {
    return state.textDocuments;
  },
  onDidChangeConfiguration: (l: Listener<any>) => {
    state.configListeners.push(l);
    return disposable;
  },
  onDidChangeTextDocument: (l: Listener<any>) => {
    state.changeListeners.push(l);
    return disposable;
  },
  onDidCloseTextDocument: () => disposable,
  registerTextDocumentContentProvider: (scheme: string, provider: any) => {
    state.contentProviders.set(scheme, provider);
    return disposable;
  },
  openTextDocument: async (uri: Uri) => ({ uri }),
};

export const commands = {
  registerCommand: (id: string, handler: (...args: any[]) => unknown) => {
    if (state.commands.has(id)) throw new Error(`command ${id} registered twice`);
    state.commands.set(id, handler);
    return disposable;
  },
  executeCommand: async (command: string, ...args: unknown[]) => {
    state.executed.push({ command, args });
  },
};

export const tasks = {
  registerTaskProvider: (type: string, provider: any) => {
    state.taskProviders.set(type, provider);
    return disposable;
  },
  executeTask: async (task: Task) => {
    state.tasks.push(task);
    return { task };
  },
  onDidEndTaskProcess: () => disposable,
};

export const languages = {
  registerCodeLensProvider: (selector: unknown, provider: any) => {
    state.codeLensProviders.push({ selector, provider });
    return disposable;
  },
};
