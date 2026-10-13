import { describe, expect, it } from 'vitest';
import { decodeView, encodeView, VIEW_SCHEME } from '../../virtualDocs';

describe('view URIs', () => {
  it('round-trip the kind and source URI', () => {
    for (const source of [
      'file:///w/main.craw',
      'file:///w/my%20file.craw',
      'untitled:Untitled-1',
      'file:///c%3A/Users/x/a.craw?weird=1#frag',
    ]) {
      for (const kind of ['syntax-tree', 'rust'] as const) {
        const parts = encodeView(kind, source);
        expect(parts.scheme).toBe(VIEW_SCHEME);
        expect(decodeView(parts.query)).toEqual({ kind, source });
      }
    }
  });

  it('name the view after the source file, with an extension that picks the language', () => {
    expect(encodeView('rust', 'file:///w/main.craw').path).toBe('/main.craw.rs');
    expect(encodeView('syntax-tree', 'file:///w/my%20file.craw').path).toBe('/my file.craw.ast');
    expect(encodeView('rust', 'untitled:Untitled-1').path).toBe('/untitled:Untitled-1.rs');
  });

  it('rejects malformed queries', () => {
    expect(decodeView('')).toBeUndefined();
    expect(decodeView('kind=bogus&source=file:///a')).toBeUndefined();
    expect(decodeView('kind=rust')).toBeUndefined();
  });
});
