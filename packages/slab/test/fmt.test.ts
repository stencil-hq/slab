import { describe, expect, test } from 'bun:test';
import { wasm } from '../src/wasm.ts';

// `slab fmt` in the npm CLI is backed by the WASM `fmt` export, which wraps
// the same `slab_syntax::format` the native CLI uses; these tests pin the
// contract the CLI command relies on.
describe('wasm fmt', () => {
   test('canonicalizes whitespace and is idempotent', () => {
      const W = wasm();
      const messy = 'col   {\n      text   "hi"\n}\n';
      const once = W.fmt(messy);
      expect(once).toContain('text "hi"');
      expect(W.fmt(once)).toBe(once);
   });

   test('returns already-canonical source unchanged', () => {
      const W = wasm();
      const canonical = W.fmt('col { text "hi" }\n');
      expect(W.fmt(canonical)).toBe(canonical);
   });
});
