// slab language support for CodeMirror 6 — a StreamLanguage parser that
// tokenizes the `.slab` surface syntax (tokens/params/def/anim/when blocks,
// node declarations, attribute `key=value` pairs, colors, comments). Token
// names are @lezer/highlight tag names; the editor must install a
// `syntaxHighlighting(...)` extension (oneDark provides one) for the classes
// to render.
//
// Keyword vocabulary from tree-sitter-slab/grammar.js.

import { LanguageSupport, StreamLanguage } from '@codemirror/language';

// Node kinds + declaration keywords.
const KEYWORDS: Record<string, true> = {
   tokens: true,
   params: true,
   def: true,
   anim: true,
   when: true,
   export: true,
   icon: true,
   each: true,
   divider: true,
   hole: true,
   col: true,
   row: true,
   wrap: true,
   grid: true,
   stack: true,
   canvas: true,
   para: true,
   group: true,
   span: true,
   text: true,
   rect: true,
   img: true,
   path: true,
   spacer: true,
};

// Boolean node flags (closed vocabulary).
const FLAGS: Record<string, true> = {
   clip: true,
   bleed: true,
   scroll: true,
   nowrap: true,
   ellipsis: true,
   inert: true,
   focusable: true,
   sticky: true,
   virtual: true,
   'drag-ghost': true,
};

// Param types + when-condition idents.
const TYPES: Record<string, true> = {
   num: true,
   pct: true,
   color: true,
   bool: true,
   enum: true,
   list: true,
   portrait: true,
   landscape: true,
   dark: true,
   coarse: true,
   web: true,
   gpu: true,
   tui: true,
   svg: true,
   png: true,
   dragging: true,
   drop: true,
   current: true,
   cross: true,
   both: true,
   'below-start': true,
   'below-center': true,
   'below-end': true,
   'above-start': true,
   'above-center': true,
   'above-end': true,
   'left-start': true,
   'left-center': true,
   'left-end': true,
   'right-start': true,
   'right-center': true,
   'right-end': true,
   auto: true,
   none: true,
   mixed: true,
   off: true,
   polite: true,
   assertive: true,
};

const slabStream = StreamLanguage.define({
   name: 'slab',
   token: (stream) => {
      // line comment
      if (stream.match('//')) {
         stream.skipToEnd();
         return 'comment';
      }
      if (stream.eatSpace()) return null;
      // string literal (no escapes in slab strings)
      if (stream.eat('"')) {
         let ch = stream.next();
         while (ch != null && ch !== '"') ch = stream.next();
         return 'string';
      }
      // hex color — 'atom' renders distinctly under oneDark
      if (stream.match(/^#[0-9A-Fa-f]{3,8}\b/)) return 'atom';
      // oklch(...) / linear(...) / radial(...) / conic(...) paint calls
      if (stream.match(/^(oklch|linear|radial|conic)\(/)) {
         let depth = 1;
         let ch = stream.next();
         while (ch != null && depth > 0) {
            if (ch === '(') depth++;
            if (ch === ')') depth--;
            if (depth === 0) break;
            ch = stream.next();
         }
         return 'atom';
      }
      // number / percentage
      if (stream.match(/^[+-]?\d+(\.\d+)?%?/)) return 'number';
      // identifier / keyword / attribute key
      if (stream.match(/^[A-Za-z_][A-Za-z0-9_]*(-[A-Za-z_][A-Za-z0-9_]*)*/)) {
         const word = stream.current();
         // `key=` → attribute name
         if (stream.peek() === '=') return 'propertyName';
         if (KEYWORDS[word]) return 'keyword';
         if (FLAGS[word]) return 'modifier';
         if (TYPES[word]) return 'typeName';
         // dotted refs (style=text.mono, param.title) — path head
         if (stream.peek() === '.') return 'namespace';
         return 'variableName';
      }
      if (stream.eat('.')) return 'punctuation';
      if (stream.match(/^[={}();,]/)) return 'punctuation';
      stream.next();
      return null;
   },
   languageData: {
      commentTokens: { line: '//' },
   },
});

/** slab language support — syntax highlighting via StreamLanguage. */
export function slab(): LanguageSupport {
   return new LanguageSupport(slabStream);
}
