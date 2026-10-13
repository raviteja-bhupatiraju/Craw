/**
 * Read-only virtual documents the extension opens beside a Craw file: its
 * syntax tree (`craw/syntaxTree`) and its generated Rust (`craw/transpile`).
 * A view URI carries the source document's URI in its query, so the
 * content provider can re-request it whenever the source changes.
 */

export const VIEW_SCHEME = 'craw-view';

export type ViewKind = 'syntax-tree' | 'rust';

const EXTENSIONS: Record<ViewKind, string> = {
  'syntax-tree': '.ast',
  rust: '.rs',
};

export interface ViewUriParts {
  scheme: string;
  path: string;
  query: string;
}

function basename(uri: string): string {
  const withoutQuery = uri.split(/[?#]/)[0];
  const last = withoutQuery.split('/').pop() || 'untitled';
  try {
    return decodeURIComponent(last);
  } catch {
    return last;
  }
}

/**
 * The view's URI parts. The path's extension picks the language VS Code
 * highlights it with (`.rs` → Rust).
 */
export function encodeView(kind: ViewKind, source: string): ViewUriParts {
  return {
    scheme: VIEW_SCHEME,
    path: `/${basename(source)}${EXTENSIONS[kind]}`,
    query: new URLSearchParams({ kind, source }).toString(),
  };
}

export function decodeView(query: string): { kind: ViewKind; source: string } | undefined {
  const params = new URLSearchParams(query);
  const kind = params.get('kind');
  const source = params.get('source');
  if ((kind !== 'syntax-tree' && kind !== 'rust') || !source) return undefined;
  return { kind, source };
}
