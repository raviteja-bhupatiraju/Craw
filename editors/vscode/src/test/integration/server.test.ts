import { ChildProcess, spawn } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import {
  createMessageConnection,
  MessageConnection,
  StreamMessageReader,
  StreamMessageWriter,
} from 'vscode-jsonrpc/node';
import { buildServerSettings } from '../../config';

/**
 * The extension's side of the protocol against the real `craw-lsp`: the
 * settings the extension would send (built from package.json's declared
 * defaults), the custom requests its commands use, and the capabilities
 * its package.json promises. Skipped when no server binary is built
 * (`cargo build --bin craw-lsp`).
 */

const repo = path.join(__dirname, '..', '..', '..', '..', '..');
const exe = process.platform === 'win32' ? 'craw-lsp.exe' : 'craw-lsp';
const binary = [path.join(repo, 'target', 'debug', exe), path.join(repo, 'target', 'release', exe)]
  .filter((p) => fs.existsSync(p))
  .sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs)[0];

const pkg = JSON.parse(fs.readFileSync(path.join(__dirname, '..', '..', '..', 'package.json'), 'utf8'));
const defaults = (key: string, fallback: unknown) =>
  pkg.contributes.configuration.properties[`craw.${key}`]?.default ?? fallback;

const URI = 'file:///integration/main.craw';
const TEXT = 'def double(n: Int):\n    return n * 2\ntotal = double(21)\nprint(totl)\n';

describe.skipIf(!binary)('craw-lsp integration', () => {
  let proc: ChildProcess;
  let conn: MessageConnection;
  let init: any;
  const diagnostics = new Map<string, any[]>();
  const waiters: (() => void)[] = [];

  beforeAll(async () => {
    proc = spawn(binary!, [], { stdio: ['pipe', 'pipe', 'ignore'] });
    conn = createMessageConnection(new StreamMessageReader(proc.stdout!), new StreamMessageWriter(proc.stdin!));
    conn.onNotification('textDocument/publishDiagnostics', (p: any) => {
      diagnostics.set(p.uri, p.diagnostics);
      waiters.splice(0).forEach((w) => w());
    });
    conn.onRequest(() => null);
    conn.listen();
    init = await conn.sendRequest('initialize', {
      processId: process.pid,
      rootUri: null,
      capabilities: {},
      initializationOptions: buildServerSettings((k, f) => defaults(k, f) as any),
    });
    await conn.sendNotification('initialized', {});
    const published = new Promise<void>((r) => waiters.push(r));
    await conn.sendNotification('textDocument/didOpen', {
      textDocument: { uri: URI, languageId: 'craw', version: 1, text: TEXT },
    });
    await published;
  }, 30_000);

  afterAll(async () => {
    try {
      await conn.sendRequest('shutdown');
      await conn.sendNotification('exit');
    } catch {
      // Already gone.
    }
    conn?.dispose();
    proc?.kill();
  });

  it('accepts the settings the extension sends and reports its version', () => {
    expect(init.serverInfo.name).toBe('craw-lsp');
    expect(init.serverInfo.version).toBe(pkg.version);
  });

  it('serves every feature the extension advertises', () => {
    const caps = init.capabilities;
    for (const key of [
      'semanticTokensProvider',
      'inlayHintProvider',
      'codeActionProvider',
      'documentFormattingProvider',
      'foldingRangeProvider',
      'selectionRangeProvider',
      'callHierarchyProvider',
      'typeDefinitionProvider',
    ]) {
      expect(caps[key], key).toBeTruthy();
    }
  });

  it('publishes diagnostics for the open document', () => {
    const d = diagnostics.get(URI)!;
    expect(d.some((x) => x.code === 'E0100' && x.message.includes('totl'))).toBe(true);
  });

  it('answers craw/syntaxTree and craw/transpile as the view commands expect', async () => {
    const tree = await conn.sendRequest<string>('craw/syntaxTree', { textDocument: { uri: URI } });
    expect(tree).toContain('FunctionDef');
    const out = await conn.sendRequest<{ rust: string }>('craw/transpile', { textDocument: { uri: URI } });
    expect(out.rust).toContain('craw_main');
  });

  it('shows all three inlay hint kinds with default settings', async () => {
    const hints: any[] = await conn.sendRequest('textDocument/inlayHint', {
      textDocument: { uri: URI },
      range: { start: { line: 0, character: 0 }, end: { line: 4, character: 0 } },
    });
    const labels = hints.map((h) => h.label);
    expect(labels).toEqual(expect.arrayContaining(['n:', ': Int', '-> Int']));
  });

  it('turns hints off when the extension pushes a settings change', async () => {
    const settings = buildServerSettings((k, f) => (k.startsWith('inlayHints') ? false : f) as any);
    await conn.sendNotification('workspace/didChangeConfiguration', { settings: { craw: settings } });
    const hints: any[] = await conn.sendRequest('textDocument/inlayHint', {
      textDocument: { uri: URI },
      range: { start: { line: 0, character: 0 }, end: { line: 4, character: 0 } },
    });
    expect(hints).toEqual([]);
  });
});
