import * as fs from 'fs';
import * as path from 'path';
import { describe, expect, it } from 'vitest';

/**
 * Keeps package.json, the extension source and the language server in
 * agreement: a command contributed but never registered (or registered
 * but not contributed), a setting read but not declared, or a semantic
 * token type the server emits but VS Code was never told about all fail
 * silently at runtime, so they are checked here instead.
 */

const root = path.join(__dirname, '..', '..', '..');
const repo = path.join(root, '..', '..');
const pkg = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'));
const source = fs
  .readdirSync(path.join(root, 'src'))
  .filter((f) => f.endsWith('.ts'))
  .map((f) => fs.readFileSync(path.join(root, 'src', f), 'utf8'))
  .join('\n');

const contributed: string[] = pkg.contributes.commands.map((c: any) => c.command);
const registered = [...source.matchAll(/'(craw\.[A-Za-z]+)':\s/g)].map((m) => m[1]);

describe('commands', () => {
  it('every contributed command is registered, and vice versa', () => {
    expect([...registered].sort()).toEqual([...contributed].sort());
  });

  it('every command has a "Craw: " title', () => {
    for (const c of pkg.contributes.commands) expect(c.title).toMatch(/^Craw: /);
  });

  it('menus, keybindings and lenses only reference contributed commands', () => {
    const referenced: string[] = [];
    for (const items of Object.values(pkg.contributes.menus) as any[][]) {
      for (const item of items) referenced.push(item.command);
    }
    for (const k of pkg.contributes.keybindings) referenced.push(k.command);
    for (const m of source.matchAll(/command: '(craw\.[A-Za-z]+)'/g)) referenced.push(m[1]);
    for (const r of referenced) expect(contributed, r).toContain(r);
  });

  it('editor-only commands are hidden from the palette outside Craw files', () => {
    const palette = new Map(pkg.contributes.menus.commandPalette.map((m: any) => [m.command, m.when]));
    for (const id of ['craw.run', 'craw.build', 'craw.transpile', 'craw.showSyntaxTree', 'craw.showTranspiledRust']) {
      expect(palette.get(id), id).toBe('editorLangId == craw');
    }
  });

  it('keybindings do not shadow VS Code built-ins', () => {
    // Ctrl+Shift+R is "Refactor…"; Ctrl+F5 is "Run Without Debugging".
    const reserved = ['ctrl+shift+r', 'cmd+shift+r', 'ctrl+f5', 'f5'];
    for (const k of pkg.contributes.keybindings) {
      expect(reserved).not.toContain(k.key);
      if (k.mac) expect(reserved).not.toContain(k.mac);
      expect(k.when).toContain('editorLangId == craw');
    }
  });
});

describe('settings', () => {
  const declared = Object.keys(pkg.contributes.configuration.properties);

  it('every setting the extension reads is declared', () => {
    const read = new Set<string>();
    for (const m of source.matchAll(/getConfiguration\('craw'\)\s*\.get(?:<[^>]+>)?\('([^']+)'/g)) read.add(`craw.${m[1]}`);
    for (const m of source.matchAll(/config\.get(?:<[^>]+>)?\('([^']+)'/g)) read.add(`craw.${m[1]}`);
    for (const m of source.matchAll(/get\('([a-zA-Z.]+)', (?:true|false)\)/g)) read.add(`craw.${m[1]}`);
    expect(read.size).toBeGreaterThan(8);
    for (const key of read) expect(declared, key).toContain(key);
  });

  it('every declared setting is used somewhere', () => {
    for (const key of declared) {
      const suffix = key.replace(/^craw\./, '');
      expect(source.includes(`'${suffix}'`) || source.includes(`'${key}'`), key).toBe(true);
    }
  });

  it('server-side settings match what craw-lsp parses', () => {
    const all = fs.readFileSync(path.join(repo, 'src', 'bin', 'craw-lsp.rs'), 'utf8');
    const start = all.indexOf('fn parse_server_config');
    const rs = all.slice(start, all.indexOf('\n}\n', start));
    const pointers = [...rs.matchAll(/"(\/[a-zA-Z/]+)"/g)].map((m) => m[1]);
    for (const p of pointers) {
      const key = 'craw.' + p.slice(1).replace(/\//g, '.');
      expect(declared, key).toContain(key);
    }
    expect(pointers.length).toBeGreaterThanOrEqual(5);
  });

  it('booleans default to true (every feature on out of the box)', () => {
    for (const [key, spec] of Object.entries<any>(pkg.contributes.configuration.properties)) {
      if (spec.type === 'boolean' && key !== 'craw.server.useWorkspaceBuild') expect(spec.default, key).toBe(true);
    }
  });
});

describe('semantic tokens', () => {
  const rs = fs.readFileSync(path.join(repo, 'src', 'ide', 'semantic_tokens.rs'), 'utf8');
  const list = (name: string) => {
    const m = rs.match(new RegExp(`pub const ${name}: &\\[&str\\] = &\\[([^\\]]*)\\]`));
    return [...m![1].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
  };
  // https://code.visualstudio.com/api/language-extensions/semantic-highlight-guide#standard-token-types-and-modifiers
  const standardTypes = [
    'namespace', 'class', 'enum', 'interface', 'struct', 'typeParameter', 'type', 'parameter', 'variable',
    'property', 'enumMember', 'decorator', 'event', 'function', 'method', 'macro', 'label', 'comment',
    'string', 'keyword', 'number', 'regexp', 'operator',
  ];
  const standardModifiers = [
    'declaration', 'definition', 'readonly', 'static', 'deprecated', 'abstract', 'async', 'modification',
    'documentation', 'defaultLibrary',
  ];

  it('custom token types the server emits are declared', () => {
    const declared = (pkg.contributes.semanticTokenTypes ?? []).map((t: any) => t.id);
    for (const t of list('TOKEN_TYPES')) {
      if (!standardTypes.includes(t)) expect(declared, t).toContain(t);
    }
  });

  it('custom modifiers the server emits are declared', () => {
    const declared = (pkg.contributes.semanticTokenModifiers ?? []).map((t: any) => t.id);
    for (const m of list('TOKEN_MODIFIERS')) {
      if (!standardModifiers.includes(m)) expect(declared, m).toContain(m);
    }
  });
});

describe('activation and packaging', () => {
  it('activates on Craw files and on workspaces containing them', () => {
    expect(pkg.activationEvents).toContain('onLanguage:craw');
    expect(pkg.activationEvents).toContain('workspaceContains:**/*.craw');
  });

  it('the formatter default names this extension', () => {
    expect(pkg.contributes.configurationDefaults['[craw]']['editor.defaultFormatter']).toBe(
      `${pkg.publisher}.${pkg.name}`,
    );
  });

  it('declares embedded Rust for the grammar', () => {
    expect(pkg.contributes.grammars[0].embeddedLanguages['meta.embedded.block.rust']).toBe('rust');
  });

  it('every file referenced from the manifest exists', () => {
    const files = [
      pkg.icon,
      pkg.contributes.languages[0].configuration,
      pkg.contributes.grammars[0].path,
      pkg.contributes.snippets[0].path,
    ];
    for (const f of files) expect(fs.existsSync(path.join(root, f)), f).toBe(true);
  });
});
