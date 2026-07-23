// slab source model — a lenient, span-preserving parser for the `.slab`
// surface syntax plus the source-edit builders the vibeviewer uses. The
// compiled SLIR is the truth for GEOMETRY (rects, hit order); this model is
// the truth for TEXT (where a node/attr lives in the editor buffer).
//
// The two meet through node KEYS: segments here replicate the compiler's
// derivation (crates/slab-compile expand.rs `segment`) — explicit `key=` →
// `#id` → `<name>@<n>` with per-sibling-scope counters — so a scene node's
// key path resolves back to the source node that authored it, including
// through component calls (expanded body roots and slot children continue
// under the call's key, SPEC §15.1).
//
// Every edit is expressed as CodeMirror-ready `{from, to, insert}` changes
// against the parsed text, so undo/redo falls out of editor history.

/** Half-open span into the parsed text. */
export interface Span {
   from: number;
   to: number;
}

/** A CodeMirror-compatible change against the text this doc was parsed from. */
export interface Change {
   from: number;
   to: number;
   insert: string;
}

export interface SrcAttr {
   name: string;
   nameSpan: Span;
   /** Value text exactly as authored (may be a tuple / color fn). */
   value: string;
   valueSpan: Span;
}

export interface SrcFlag {
   name: string;
   span: Span;
}

export type ArgKind = 'string' | 'number' | 'percent' | 'ident' | 'ref' | 'color';

export interface SrcArg {
   kind: ArgKind;
   /** Raw text incl. quotes for strings. */
   text: string;
   span: Span;
}

export interface SrcWhen {
   cond: string;
   span: Span;
   attrs: SrcAttr[];
   flags: SrcFlag[];
   line: number;
}

export interface SrcNode {
   /** Node name: lowercase builtin, Capitalized component call, or 'span'
    * for bare string children (the compiler synthesizes span nodes). */
   kind: string;
   isCall: boolean;
   /** `#id` without the hash, when present. */
   id: string | null;
   idSpan: Span | null;
   nameSpan: Span;
   /** Name through the last header item (attrs/flags/args), excl. block. */
   headerSpan: Span;
   /** Whole node incl. block braces. */
   span: Span;
   /** Inside the braces (exclusive), when a block exists. */
   bodySpan: Span | null;
   /** 1-based source line of the name token. */
   line: number;
   /** Leading whitespace of the node's line ('' when not first on line). */
   indent: string;
   /** True when the node starts its line (safe to delete whole lines). */
   ownLine: boolean;
   attrs: SrcAttr[];
   flags: SrcFlag[];
   args: SrcArg[];
   whens: SrcWhen[];
   children: SrcNode[];
   parent: SrcNode | null;
   /** Key segment per the compiler's derivation, assigned post-parse. */
   seg: string;
}

export interface SrcDefField {
   name: string;
   /** Default exactly as authored; `list(Name)` carries nested schema identity. */
   def: string;
}

export interface SrcDef {
   name: string;
   params: string[];
   fields: SrcDefField[];
   exported: boolean;
   body: SrcNode[];
   span: Span;
   line: number;
}

export interface SrcParam {
   name: string;
   type: string;
   def: string;
   line: number;
}

/** A detached top-level icon declaration, excluded from layout roots. */
export interface SrcIcon {
   name: string;
   node: SrcNode;
   span: Span;
   line: number;
}

/** Structural mirror of SlabElement's generated list-schema surface. */
export interface SrcListFieldSchema {
   name: string;
   type: number;
   sub: number;
   enum?: readonly string[];
}

export interface SrcListRowSchema {
   name: string;
   fields: SrcListFieldSchema[];
}

export interface SrcListSchema extends SrcListRowSchema {
   param: number;
   row: number;
}
/** One `<pct> { attrs }` stop inside an `anim` block. */
export interface SrcKeyframe {
   pct: number;
   pctSpan: Span;
   /** Inside the stop's braces (exclusive) — raw attr text. */
   bodySpan: Span;
   span: Span;
}

export interface SrcAnim {
   name: string;
   nameSpan: Span;
   keyframes: SrcKeyframe[];
   span: Span;
   /** Offset of the closing `}` — keyframe insertion point. */
   closeAt: number;
   line: number;
}

export interface SrcDoc {
   text: string;
   /** Document-tree top-level nodes (defs and detached icons excluded). */
   roots: SrcNode[];
   defs: Map<string, SrcDef>;
   icons: SrcIcon[];
   params: SrcParam[];
   anims: SrcAnim[];
   listSchemas: Record<string, SrcListSchema>;
   listSchemaRows: SrcListRowSchema[];
   /** Dotted token paths (`color.bg`, `text.title`, …) for suggestions. */
   tokenPaths: string[];
   /** Indent unit inferred from the source (default two spaces). */
   indentUnit: string;
}

/** Resolution of a scene key back to source: the node, and the def it lives
 * in when the key crossed a component call (edits then target the def body,
 * i.e. every instance — component-master semantics). */
export interface SrcTarget {
   node: SrcNode;
   def: string | null;
}

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

/** Node kinds that lay out children (insertion targets in the palette). */
export const CONTAINERS: Record<string, true> = {
   row: true,
   col: true,
   wrap: true,
   grid: true,
   stack: true,
   canvas: true,
   para: true,
   group: true,
};

// ── tokenizer ────────────────────────────────────────────────────────

type TokType = 'nl' | 'ident' | 'ref' | 'number' | 'percent' | 'string' | 'hash' | 'punct';

interface Tok {
   t: TokType;
   from: number;
   to: number;
   text: string;
}

const isIdentStart = (c: string) => /[A-Za-z_]/.test(c);
const isIdentPart = (c: string) => /[A-Za-z0-9_]/.test(c);

function tokenize(text: string): Tok[] {
   const toks: Tok[] = [];
   let i = 0;
   const n = text.length;
   while (i < n) {
      const c = text[i];
      // line continuation swallows the newline
      if (c === '\\' && (text[i + 1] === '\n' || (text[i + 1] === '\r' && text[i + 2] === '\n'))) {
         i += text[i + 1] === '\r' ? 3 : 2;
         continue;
      }
      if (c === ' ' || c === '\t' || c === '\r') {
         i++;
         continue;
      }
      if (c === '\n' || c === ';') {
         toks.push({ t: 'nl', from: i, to: i + 1, text: c });
         i++;
         continue;
      }
      if (c === '/' && text[i + 1] === '/') {
         while (i < n && text[i] !== '\n') i++;
         continue;
      }
      if (c === '/' && text[i + 1] === '*') {
         const end = text.indexOf('*/', i + 2);
         i = end === -1 ? n : end + 2;
         continue;
      }
      if (c === '"') {
         const from = i;
         i++;
         while (i < n && text[i] !== '"') {
            if (text[i] === '\\') i++;
            i++;
         }
         i = Math.min(i + 1, n);
         toks.push({ t: 'string', from, to: i, text: text.slice(from, i) });
         continue;
      }
      if (c === '#') {
         const from = i;
         i++;
         while (i < n && /[0-9A-Za-z_-]/.test(text[i])) i++;
         toks.push({ t: 'hash', from, to: i, text: text.slice(from, i) });
         continue;
      }
      if (
         /[0-9]/.test(c) ||
         (c === '-' && /[0-9.]/.test(text[i + 1] ?? '')) ||
         (c === '.' && /[0-9]/.test(text[i + 1] ?? ''))
      ) {
         const from = i;
         if (text[i] === '-') i++;
         while (i < n && /[0-9]/.test(text[i])) i++;
         if (text[i] === '.') {
            i++;
            while (i < n && /[0-9]/.test(text[i])) i++;
         }
         let t: TokType = 'number';
         if (text[i] === '%') {
            i++;
            t = 'percent';
         }
         toks.push({ t, from, to: i, text: text.slice(from, i) });
         continue;
      }
      if (isIdentStart(c)) {
         const from = i;
         i++;
         while (i < n) {
            if (isIdentPart(text[i])) {
               i++;
            } else if (text[i] === '-' && isIdentStart(text[i + 1] ?? '')) {
               i++;
            } else if (text[i] === '.' && isIdentStart(text[i + 1] ?? '')) {
               i++; // dotted reference
            } else {
               break;
            }
         }
         const t = text.slice(from, i);
         toks.push({ t: t.includes('.') ? 'ref' : 'ident', from, to: i, text: t });
         continue;
      }
      toks.push({ t: 'punct', from: i, to: i + 1, text: c });
      i++;
   }
   toks.push({ t: 'nl', from: n, to: n, text: '\n' });
   return toks;
}

// ── parser ───────────────────────────────────────────────────────────

class Parser {
   toks: Tok[];
   pos = 0;
   text: string;
   lineStarts: number[];

   constructor(text: string) {
      this.text = text;
      this.toks = tokenize(text);
      this.lineStarts = [0];
      for (let i = 0; i < text.length; i++) {
         if (text[i] === '\n') this.lineStarts.push(i + 1);
      }
   }

   peek(): Tok {
      return this.toks[Math.min(this.pos, this.toks.length - 1)];
   }

   next(): Tok {
      const t = this.peek();
      this.pos = Math.min(this.pos + 1, this.toks.length - 1);
      return t;
   }

   atEnd(): boolean {
      return this.pos >= this.toks.length - 1;
   }

   skipNl(): void {
      while (!this.atEnd() && this.peek().t === 'nl') this.next();
   }

   lineOf(offset: number): number {
      let lo = 0;
      let hi = this.lineStarts.length - 1;
      while (lo < hi) {
         const mid = (lo + hi + 1) >> 1;
         if (this.lineStarts[mid] <= offset) lo = mid;
         else hi = mid - 1;
      }
      return lo + 1;
   }

   indentOf(offset: number): { indent: string; ownLine: boolean } {
      const ls = this.lineStarts[this.lineOf(offset) - 1];
      const prefix = this.text.slice(ls, offset);
      return /^[ \t]*$/.test(prefix)
         ? { indent: prefix, ownLine: true }
         : { indent: '', ownLine: false };
   }

   /** Skip a balanced `{ … }` block (already positioned before `{`). */
   skipBalanced(): void {
      if (this.peek().text !== '{') return;
      let depth = 0;
      while (!this.atEnd()) {
         const t = this.next();
         if (t.text === '{') depth++;
         else if (t.text === '}') {
            depth--;
            if (depth === 0) return;
         }
      }
   }

   /** One source value, including tuples and recursively balanced list literals. */
   value(stopAtDefParam = false): Span | null {
      const start = this.peek();
      let last: Tok | null = null;
      for (;;) {
         const s = this.scalar();
         if (!s) break;
         last = s;
         if (this.peek().text === ',') {
            if (stopAtDefParam && this.defParamFollows()) break;
            this.next();
            continue;
         }
         break;
      }
      return last ? { from: start.from, to: last.to } : null;
   }

   /** True when the current comma begins the next `def` parameter. */
   defParamFollows(): boolean {
      let at = this.pos + 1;
      while (this.toks[at]?.t === 'nl') at++;
      if (this.toks[at]?.t !== 'ident') return false;
      at++;
      while (this.toks[at]?.t === 'nl') at++;
      const next = this.toks[at]?.text;
      return next === '=' || next === ',' || next === ')';
   }

   /** One scalar; returns its LAST token (for span ends). */
   scalar(): Tok | null {
      const t = this.peek();
      if (t.t === 'nl' || t.text === '{' || t.text === '}') return null;
      if (t.text === '[') {
         let depth = 0;
         let last = t;
         while (!this.atEnd()) {
            const part = this.next();
            last = part;
            if (part.text === '[') depth++;
            else if (part.text === ']') {
               depth--;
               if (depth === 0) break;
            }
         }
         return last;
      }
      if (
         t.t === 'number' ||
         t.t === 'percent' ||
         t.t === 'string' ||
         t.t === 'hash' ||
         t.t === 'ref'
      ) {
         return this.next();
      }
      if (t.t === 'ident') {
         const id = this.next();
         // fill:2
         if (this.peek().text === ':' && this.toks[this.pos + 1]?.t === 'number') {
            this.next();
            return this.next();
         }
         // color fn: ident( … balanced … )
         if (this.peek().text === '(') {
            let depth = 0;
            let last = id;
            while (!this.atEnd()) {
               const p = this.next();
               last = p;
               if (p.text === '(') depth++;
               else if (p.text === ')') {
                  depth--;
                  if (depth === 0) break;
               }
            }
            return last;
         }
         return id;
      }
      return null;
   }
}

function parseNode(p: Parser, parent: SrcNode | null): SrcNode {
   const name = p.next(); // ident
   const { indent, ownLine } = p.indentOf(name.from);
   const node: SrcNode = {
      kind: name.text,
      isCall: /[A-Z]/.test(name.text[0]),
      id: null,
      idSpan: null,
      nameSpan: { from: name.from, to: name.to },
      headerSpan: { from: name.from, to: name.to },
      span: { from: name.from, to: name.to },
      bodySpan: null,
      line: p.lineOf(name.from),
      indent,
      ownLine,
      attrs: [],
      flags: [],
      args: [],
      whens: [],
      children: [],
      parent,
      seg: '',
   };
   // header items until newline / { / }
   for (;;) {
      const t = p.peek();
      if (t.t === 'nl' || t.text === '{' || t.text === '}' || p.atEnd()) break;
      if (t.t === 'hash') {
         const h = p.next();
         if (node.id === null && node.attrs.length === 0 && node.args.length === 0) {
            node.id = h.text.slice(1);
            node.idSpan = { from: h.from, to: h.to };
         } else {
            node.args.push({ kind: 'color', text: h.text, span: { from: h.from, to: h.to } });
         }
         node.headerSpan.to = h.to;
         continue;
      }
      if (t.t === 'ident') {
         const isAttr = p.toks[p.pos + 1]?.text === '=';
         if (isAttr) {
            const nameTok = p.next();
            p.next(); // =
            const vspan = p.value();
            const span = vspan ?? { from: p.peek().from, to: p.peek().from };
            node.attrs.push({
               name: nameTok.text,
               nameSpan: { from: nameTok.from, to: nameTok.to },
               value: p.text.slice(span.from, span.to),
               valueSpan: span,
            });
            node.headerSpan.to = span.to;
            continue;
         }
         if (FLAGS[t.text] === true) {
            const f = p.next();
            node.flags.push({ name: f.text, span: { from: f.from, to: f.to } });
            node.headerSpan.to = f.to;
            continue;
         }
         // bare ident arg (prop ref, keyword, hole name); may be fill:2 / fn(...)
         const last = p.scalar();
         if (last) {
            node.args.push({
               kind: 'ident',
               text: p.text.slice(t.from, last.to),
               span: { from: t.from, to: last.to },
            });
            node.headerSpan.to = last.to;
            continue;
         }
         p.next();
         continue;
      }
      if (t.t === 'string' || t.t === 'number' || t.t === 'percent' || t.t === 'ref') {
         const a = p.next();
         const kind: ArgKind =
            a.t === 'string'
               ? 'string'
               : a.t === 'ref'
                 ? 'ref'
                 : a.t === 'percent'
                   ? 'percent'
                   : 'number';
         node.args.push({ kind, text: a.text, span: { from: a.from, to: a.to } });
         node.headerSpan.to = a.to;
         continue;
      }
      p.next(); // unknown punct — skip
   }
   node.span.to = node.headerSpan.to;
   // optional block
   if (p.peek().text === '{') {
      const open = p.next();
      node.bodySpan = { from: open.to, to: open.to };
      parseChildren(p, node);
      // parseChildren stops at `}` or EOF
      if (p.peek().text === '}') {
         const close = p.next();
         node.bodySpan.to = close.from;
         node.span.to = close.to;
      } else {
         node.bodySpan.to = p.peek().from;
         node.span.to = p.peek().from;
      }
   }
   return node;
}

function parseChildren(p: Parser, parent: SrcNode): void {
   for (;;) {
      p.skipNl();
      const t = p.peek();
      if (t.text === '}' || p.atEnd()) return;
      if (t.t === 'ident' && t.text === 'when') {
         parseWhen(p, parent);
         continue;
      }
      if (t.t === 'ident' || (t.t === 'ref' && false)) {
         parent.children.push(parseNode(p, parent));
         continue;
      }
      if (t.t === 'string') {
         // bare text run — the compiler synthesizes a span node for it
         const s = p.next();
         const { indent, ownLine } = p.indentOf(s.from);
         parent.children.push({
            kind: 'span',
            isCall: false,
            id: null,
            idSpan: null,
            nameSpan: { from: s.from, to: s.to },
            headerSpan: { from: s.from, to: s.to },
            span: { from: s.from, to: s.to },
            bodySpan: null,
            line: p.lineOf(s.from),
            indent,
            ownLine,
            attrs: [],
            flags: [],
            args: [{ kind: 'string', text: s.text, span: { from: s.from, to: s.to } }],
            whens: [],
            children: [],
            parent,
            seg: '',
         });
         continue;
      }
      p.next(); // stray token — skip
   }
}

function parseWhen(p: Parser, parent: SrcNode): void {
   const kw = p.next(); // when
   // condition: raw tokens until `{`
   let condEnd = kw.to;
   while (!p.atEnd() && p.peek().text !== '{' && p.peek().t !== 'nl') {
      condEnd = p.next().to;
   }
   const cond = p.text.slice(kw.to, condEnd).trim();
   p.skipNl();
   const when: SrcWhen = {
      cond,
      span: { from: kw.from, to: condEnd },
      attrs: [],
      flags: [],
      line: p.lineOf(kw.from),
   };
   if (p.peek().text === '{') {
      p.next();
      for (;;) {
         p.skipNl();
         const t = p.peek();
         if (t.text === '}' || p.atEnd()) break;
         if (t.t === 'ident' && p.toks[p.pos + 1]?.text === '=') {
            const nameTok = p.next();
            p.next();
            const vspan = p.value();
            if (vspan) {
               when.attrs.push({
                  name: nameTok.text,
                  nameSpan: { from: nameTok.from, to: nameTok.to },
                  value: p.text.slice(vspan.from, vspan.to),
                  valueSpan: vspan,
               });
            }
            continue;
         }
         if (t.t === 'ident' && FLAGS[t.text] === true) {
            const f = p.next();
            when.flags.push({ name: f.text, span: { from: f.from, to: f.to } });
            continue;
         }
         if (t.t === 'ident' && t.text === 'when') {
            parseWhen(p, parent);
            continue;
         }
         if (t.t === 'ident') {
            // extra children appended in place — they join the sibling scope
            parent.children.push(parseNode(p, parent));
            continue;
         }
         if (t.t === 'string') {
            p.next();
            continue;
         }
         p.next();
      }
      if (p.peek().text === '}') {
         const close = p.next();
         when.span.to = close.to;
      }
   }
   parent.whens.push(when);
   parent.span.to = Math.max(parent.span.to, when.span.to);
}

/** Compiler-faithful sibling key segments (expand.rs `segment`): explicit
 * `key=` → `#id` → `name@n`, counters per name over UNKEYED nodes only. */
function assignSegments(list: SrcNode[]): void {
   const counters = new Map<string, number>();
   for (const c of list) {
      const keyAttr = c.attrs.find((a) => a.name === 'key');
      if (keyAttr) {
         c.seg = keyAttr.value.replace(/^"|"$/g, '');
      } else if (c.id !== null) {
         c.seg = `#${c.id}`;
      } else {
         const n = counters.get(c.kind) ?? 0;
         counters.set(c.kind, n + 1);
         c.seg = `${c.kind}@${n}`;
      }
      assignSegments(c.children);
   }
}

const TY_TEXT = 0;
const TY_NUM = 1;
const TY_COLOR = 3;
const TY_BOOL = 4;
const TY_LIST = 6;

const INFER_COLOR_ATTRS: Record<string, true> = {
   bg: true,
   stroke: true,
   color: true,
   mask: true,
   'backdrop-mask': true,
};
const INFER_TEXT_ATTRS: Record<string, true> = { act: true, field: true, src: true };
const INFER_NUM_ATTRS: Record<string, true> = {
   w: true,
   h: true,
   'min-w': true,
   'max-w': true,
   'min-h': true,
   'max-h': true,
   size: true,
   weight: true,
   gap: true,
   radius: true,
   'stroke-w': true,
   opacity: true,
   tracking: true,
   leading: true,
   blur: true,
   rotate: true,
   span: true,
   scale: true,
   smooth: true,
};
const INFER_NUM_TUPLE_ATTRS: Record<string, true> = {
   pad: true,
   offset: true,
   at: true,
   cols: true,
   'stroke-dash': true,
   scale: true,
   grain: true,
   tilt: true,
};

function listSchemaName(value: string): string | null {
   return /^list\s*\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*\)$/.exec(value.trim())?.[1] ?? null;
}

/** Split a source tuple without treating nested calls/lists as tuple separators. */
function sourceTuple(value: string): string[] {
   const parts: string[] = [];
   let start = 0;
   let round = 0;
   let square = 0;
   let quoted = false;
   let escaped = false;
   for (let i = 0; i < value.length; i++) {
      const c = value[i];
      if (quoted) {
         if (escaped) escaped = false;
         else if (c === '\\') escaped = true;
         else if (c === '"') quoted = false;
         continue;
      }
      if (c === '"') quoted = true;
      else if (c === '(') round++;
      else if (c === ')') round--;
      else if (c === '[') square++;
      else if (c === ']') square--;
      else if (c === ',' && round === 0 && square === 0) {
         parts.push(value.slice(start, i).trim());
         start = i + 1;
      }
   }
   parts.push(value.slice(start).trim());
   return parts;
}

interface InferredField {
   name: string;
   type: number;
   schema: string | null;
}

/** Mirror the compiler's exported-def prop inference for SlabElement.setList. */
function inferDefFields(def: SrcDef): InferredField[] {
   const votes = new Map<
      string,
      { type: number | null; conflict: boolean; schema: string | null }
   >();
   for (const field of def.fields) {
      const schema = listSchemaName(field.def);
      votes.set(field.name, {
         type: schema ? TY_LIST : null,
         conflict: false,
         schema,
      });
   }
   const cast = (name: string, type: number): void => {
      const vote = votes.get(name);
      if (!vote) return;
      if (vote.type === null) vote.type = type;
      else if (vote.type !== type) vote.conflict = true;
   };
   const attrs = (items: readonly SrcAttr[]): void => {
      for (const attr of items) {
         const type = INFER_COLOR_ATTRS[attr.name]
            ? TY_COLOR
            : INFER_TEXT_ATTRS[attr.name]
              ? TY_TEXT
              : INFER_NUM_ATTRS[attr.name] || INFER_NUM_TUPLE_ATTRS[attr.name]
                ? TY_NUM
                : null;
         if (type === null) continue;
         for (const part of sourceTuple(attr.value)) cast(part, type);
      }
   };
   const walk = (nodes: readonly SrcNode[]): void => {
      for (const node of nodes) {
         if (node.kind === 'text' || node.kind === 'span' || node.kind === 'para') {
            for (const arg of node.args) {
               if (arg.kind === 'ident') cast(arg.text, TY_TEXT);
            }
         }
         attrs(node.attrs);
         for (const when of node.whens) {
            const condition = /^!?([A-Za-z_][A-Za-z0-9_]*)$/.exec(when.cond.trim());
            if (condition) cast(condition[1], TY_BOOL);
            attrs(when.attrs);
         }
         walk(node.children);
      }
   };
   walk(def.body);
   return def.fields.map((field) => {
      const vote = votes.get(field.name);
      return {
         name: field.name,
         type: !vote || vote.conflict ? TY_TEXT : (vote.type ?? TY_TEXT),
         schema: vote?.schema ?? null,
      };
   });
}

function buildListSchemas(
   defs: ReadonlyMap<string, SrcDef>,
   params: readonly SrcParam[],
): {
   listSchemas: Record<string, SrcListSchema>;
   listSchemaRows: SrcListRowSchema[];
} {
   const listSchemas: Record<string, SrcListSchema> = {};
   const listSchemaRows: SrcListRowSchema[] = [];
   const rowsByName = new Map<string, number>();

   const ensure = (name: string): number => {
      const existing = rowsByName.get(name);
      if (existing !== undefined) return existing;
      const def = defs.get(name);
      if (!def?.exported) return -1;
      const row = listSchemaRows.length;
      rowsByName.set(name, row);
      const schema: SrcListRowSchema = { name, fields: [] };
      listSchemaRows.push(schema);
      schema.fields = inferDefFields(def).map((field) => {
         let sub = 0;
         if (field.type === TY_LIST && field.schema) {
            const nested = ensure(field.schema);
            if (nested >= 0) sub = nested + 1;
         }
         return { name: field.name, type: field.type, sub };
      });
      return row;
   };

   params.forEach((param, index) => {
      const name = listSchemaName(param.type);
      if (!name) return;
      const row = ensure(name);
      if (row < 0) return;
      listSchemas[param.name] = {
         name,
         param: index,
         row,
         fields: listSchemaRows[row].fields,
      };
   });
   return { listSchemas, listSchemaRows };
}

const LIST_PARSE_FAIL = Symbol('list-parse-fail');

class ListDefaultParser {
   readonly toks: Tok[];
   pos = 0;

   constructor(text: string) {
      this.toks = tokenize(text);
   }

   skipNl(): void {
      while (
         this.toks[this.pos]?.t === 'nl' &&
         this.toks[this.pos]?.to > this.toks[this.pos]?.from
      ) {
         this.pos++;
      }
   }

   parse(): unknown | typeof LIST_PARSE_FAIL {
      this.skipNl();
      const token = this.toks[this.pos];
      if (!token) return LIST_PARSE_FAIL;
      if (token.text === '[') return this.array();
      if (token.t === 'string') {
         this.pos++;
         try {
            return JSON.parse(token.text) as unknown;
         } catch {
            return token.text.slice(1, -1);
         }
      }
      if (token.t === 'number') {
         this.pos++;
         return Number(token.text);
      }
      if (token.t === 'percent' || token.t === 'hash' || token.t === 'ref') {
         this.pos++;
         return token.text;
      }
      if (token.t !== 'ident') return LIST_PARSE_FAIL;
      this.pos++;
      if (token.text === 'true') return true;
      if (token.text === 'false') return false;
      if (this.toks[this.pos]?.text !== '(') return token.text;
      this.pos++;
      const item: Record<string, unknown> = {};
      for (;;) {
         this.skipNl();
         if (this.toks[this.pos]?.text === ')') {
            this.pos++;
            return item;
         }
         const field = this.toks[this.pos];
         if (field?.t !== 'ident' || this.toks[this.pos + 1]?.text !== '=') {
            return LIST_PARSE_FAIL;
         }
         this.pos += 2;
         const value = this.parse();
         if (value === LIST_PARSE_FAIL) return value;
         item[field.text] = value;
         this.skipNl();
         if (this.toks[this.pos]?.text === ',') {
            this.pos++;
            continue;
         }
         if (this.toks[this.pos]?.text !== ')') return LIST_PARSE_FAIL;
      }
   }

   array(): unknown[] | typeof LIST_PARSE_FAIL {
      this.pos++;
      const values: unknown[] = [];
      for (;;) {
         this.skipNl();
         if (this.toks[this.pos]?.text === ']') {
            this.pos++;
            return values;
         }
         const value = this.parse();
         if (value === LIST_PARSE_FAIL) return value;
         values.push(value);
         this.skipNl();
         if (this.toks[this.pos]?.text === ',') {
            this.pos++;
            continue;
         }
         if (this.toks[this.pos]?.text !== ']') return LIST_PARSE_FAIL;
      }
   }
}

/** Decode an authored `[Def(field=…), …]` default for the live list controls. */
export function parseListDefault(value: string): unknown[] | null {
   if (!value.trim().startsWith('[')) return null;
   const parsed = new ListDefaultParser(value).parse();
   return Array.isArray(parsed) ? parsed : null;
}

/** Parse `.slab` text into the span-preserving source model. Lenient: never
 * throws on malformed input; unknown tokens are skipped. */
export function parseSlab(text: string): SrcDoc {
   const p = new Parser(text);
   const roots: SrcNode[] = [];
   const defs = new Map<string, SrcDef>();
   const icons: SrcIcon[] = [];
   const params: SrcParam[] = [];
   const anims: SrcAnim[] = [];
   const tokenPaths: string[] = [];

   const parseTokens = (prefix: string[]): void => {
      // positioned before `{`
      if (p.peek().text !== '{') return;
      p.next();
      for (;;) {
         p.skipNl();
         const t = p.peek();
         if (t.text === '}' || p.atEnd()) break;
         if (t.t === 'ident') {
            const name = p.next();
            p.skipNl();
            if (p.peek().text === '{') {
               parseTokens([...prefix, name.text]);
            } else {
               p.value();
               if (prefix.length > 0) tokenPaths.push([...prefix, name.text].join('.'));
            }
            continue;
         }
         p.next();
      }
      if (p.peek().text === '}') p.next();
   };

   for (;;) {
      p.skipNl();
      if (p.atEnd()) break;
      const t = p.peek();
      if (t.t !== 'ident') {
         if (t.t === 'string') {
            p.next();
            continue;
         }
         if (t.text === '{') {
            p.skipBalanced();
            continue;
         }
         p.next();
         continue;
      }
      if (t.text === 'tokens') {
         p.next();
         p.skipNl();
         parseTokens([]);
         continue;
      }
      if (t.text === 'params') {
         p.next();
         p.skipNl();
         if (p.peek().text === '{') {
            p.next();
            for (;;) {
               p.skipNl();
               const pt = p.peek();
               if (pt.text === '}' || p.atEnd()) break;
               if (pt.t === 'ident') {
                  const name = p.next();
                  let type = '';
                  if (p.peek().t === 'ident') {
                     type = p.next().text;
                     if (p.peek().text === '(') {
                        const from = p.peek().from;
                        let depth = 0;
                        while (!p.atEnd()) {
                           const x = p.next();
                           if (x.text === '(') depth++;
                           else if (x.text === ')') {
                              depth--;
                              if (depth === 0) break;
                           }
                        }
                        type += p.text.slice(from, p.toks[p.pos - 1].to);
                     }
                  }
                  let def = '';
                  if (p.peek().text === '=') {
                     p.next();
                     const v = p.value();
                     if (v) def = p.text.slice(v.from, v.to);
                  }
                  params.push({ name: name.text, type, def, line: p.lineOf(name.from) });
                  continue;
               }
               p.next();
            }
            if (p.peek().text === '}') p.next();
         }
         continue;
      }
      if (t.text === 'def') {
         const kw = p.next();
         const nameTok = p.peek().t === 'ident' ? p.next() : null;
         const fields: SrcDefField[] = [];
         if (p.peek().text === '(') {
            p.next();
            while (!p.atEnd()) {
               p.skipNl();
               if (p.peek().text === ')') {
                  p.next();
                  break;
               }
               if (p.peek().text === ',') {
                  p.next();
                  continue;
               }
               if (p.peek().t !== 'ident') {
                  p.next();
                  continue;
               }
               const field = p.next();
               let def = '';
               if (p.peek().text === '=') {
                  p.next();
                  const value = p.value(true);
                  if (value) def = p.text.slice(value.from, value.to);
               }
               fields.push({ name: field.text, def });
            }
         }
         const paramNames = fields.map((field) => field.name);
         let exported = false;
         if (p.peek().t === 'ident' && p.peek().text === 'export') {
            exported = true;
            p.next();
         }
         p.skipNl();
         const holder: SrcNode = {
            kind: '__def',
            isCall: false,
            id: null,
            idSpan: null,
            nameSpan: { from: kw.from, to: kw.to },
            headerSpan: { from: kw.from, to: kw.to },
            span: { from: kw.from, to: kw.to },
            bodySpan: null,
            line: p.lineOf(kw.from),
            indent: '',
            ownLine: true,
            attrs: [],
            flags: [],
            args: [],
            whens: [],
            children: [],
            parent: null,
            seg: '',
         };
         let end = kw.to;
         if (p.peek().text === '{') {
            p.next();
            parseChildren(p, holder);
            if (p.peek().text === '}') end = p.next().to;
         }
         if (nameTok) {
            for (const c of holder.children) c.parent = null;
            defs.set(nameTok.text, {
               name: nameTok.text,
               params: paramNames,
               fields,
               exported,
               body: holder.children,
               span: { from: kw.from, to: end },
               line: p.lineOf(kw.from),
            });
         }
         continue;
      }
      if (t.text === 'anim') {
         const kw = p.next();
         const nameTok = p.peek().t === 'ident' ? p.next() : null;
         p.skipNl();
         const kfs: SrcKeyframe[] = [];
         let end = kw.to;
         let closeAt = kw.to;
         if (p.peek().text === '{') {
            p.next();
            for (;;) {
               p.skipNl();
               const kt = p.peek();
               if (kt.text === '}' || p.atEnd()) break;
               if (kt.t === 'percent') {
                  const pct = p.next();
                  p.skipNl();
                  let bodySpan: Span = { from: pct.to, to: pct.to };
                  let kfEnd = pct.to;
                  if (p.peek().text === '{') {
                     const open = p.next();
                     let depth = 1;
                     let last = open;
                     while (!p.atEnd() && depth > 0) {
                        const x = p.next();
                        if (x.text === '{') depth++;
                        else if (x.text === '}') depth--;
                        last = x;
                     }
                     bodySpan = { from: open.to, to: last.from };
                     kfEnd = last.to;
                  }
                  kfs.push({
                     pct: Number.parseFloat(pct.text),
                     pctSpan: { from: pct.from, to: pct.to },
                     bodySpan,
                     span: { from: pct.from, to: kfEnd },
                  });
                  continue;
               }
               p.next();
            }
            if (p.peek().text === '}') {
               const close = p.next();
               closeAt = close.from;
               end = close.to;
            }
         }
         if (nameTok) {
            anims.push({
               name: nameTok.text,
               nameSpan: { from: nameTok.from, to: nameTok.to },
               keyframes: kfs,
               span: { from: kw.from, to: end },
               closeAt,
               line: p.lineOf(kw.from),
            });
         }
         continue;
      }
      if (t.text === 'icon') {
         const declaration = parseNode(p, null);
         if (declaration.bodySpan) {
            const first = declaration.args[0];
            const name = first?.text.replace(/^"|"$/g, '') ?? '';
            if (name !== '') {
               icons.push({
                  name,
                  node: declaration,
                  span: declaration.span,
                  line: declaration.line,
               });
            }
         } else {
            roots.push(declaration);
         }
         continue;
      }
      if (t.text === 'when') {
         // top-level when: skip to and over the balanced block
         p.next();
         while (!p.atEnd() && p.peek().text !== '{' && p.peek().t !== 'nl') p.next();
         p.skipNl();
         p.skipBalanced();
         continue;
      }
      roots.push(parseNode(p, null));
   }

   assignSegments(roots);
   for (const d of defs.values()) assignSegments(d.body);
   for (const icon of icons) assignSegments([icon.node]);

   // indent unit: first parent→child indent delta found
   let indentUnit = '  ';
   const findUnit = (nodes: SrcNode[]): string | null => {
      for (const nd of nodes) {
         for (const c of nd.children) {
            if (c.ownLine && nd.ownLine && c.indent.length > nd.indent.length) {
               return c.indent.slice(nd.indent.length);
            }
         }
         const deep = findUnit(nd.children);
         if (deep) return deep;
      }
      return null;
   };
   indentUnit = findUnit(roots) ?? '  ';
   const { listSchemas, listSchemaRows } = buildListSchemas(defs, params);

   return {
      text,
      roots,
      defs,
      icons,
      params,
      anims,
      listSchemas,
      listSchemaRows,
      tokenPaths,
      indentUnit,
   };
}

// ── key resolution ───────────────────────────────────────────────────

/** Resolve a scene key path (`col@0/#save/text@0`) to its source node,
 * traversing component calls into def bodies (edits there hit the def —
 * every instance). Returns null when the path doesn't map (synth nodes,
 * mid-edit drift). */
export function resolveKey(doc: SrcDoc, key: string): SrcTarget | null {
   if (key === '') return null;
   const segs = key.split('/');
   let list = doc.roots;
   const def: string | null = null;
   let found: SrcNode | null = null;
   for (let i = 0; i < segs.length; i++) {
      const seg = segs[i];
      found = list.find((c) => c.seg === seg) ?? null;
      if (!found) return null;
      if (i === segs.length - 1) break;
      if (found.isCall) {
         const d = doc.defs.get(found.kind);
         // body roots and slot children both continue under the call's key;
         // try the def body first, then the call's own (slotted) children.
         if (d) {
            const rest = segs.slice(i + 1).join('/');
            const viaBody = resolveIn(doc, d.body, rest, d.name);
            if (viaBody) return viaBody;
         }
         list = found.children;
      } else {
         list = found.children;
      }
   }
   if (!found) return null;
   if (found.isCall) {
      // selecting the instance root selects the call site
      return { node: found, def };
   }
   return { node: found, def };
}

function resolveIn(doc: SrcDoc, body: SrcNode[], key: string, defName: string): SrcTarget | null {
   const segs = key.split('/');
   let list = body;
   let found: SrcNode | null = null;
   for (let i = 0; i < segs.length; i++) {
      found = list.find((c) => c.seg === segs[i]) ?? null;
      if (!found) return null;
      if (i === segs.length - 1) break;
      if (found.isCall) {
         const d = doc.defs.get(found.kind);
         if (d) {
            const via = resolveIn(doc, d.body, segs.slice(i + 1).join('/'), d.name);
            if (via) return via;
         }
      }
      list = found.children;
   }
   return found ? { node: found, def: defName } : null;
}

/** Full key path for a document-tree node (root nodes have bare segments). */
export function keyForNode(node: SrcNode): string {
   const segs: string[] = [];
   let cur: SrcNode | null = node;
   while (cur) {
      segs.unshift(cur.seg);
      cur = cur.parent;
   }
   return segs.join('/');
}

/** Deepest document-tree node whose span contains `pos` (editor→canvas). */
export function nodeAtPos(doc: SrcDoc, pos: number): SrcNode | null {
   let best: SrcNode | null = null;
   const walk = (list: SrcNode[]): void => {
      for (const nd of list) {
         if (pos >= nd.span.from && pos <= nd.span.to) {
            best = nd;
            walk(nd.children);
         }
      }
   };
   walk(doc.roots);
   return best;
}

/** First node declared on `line` (1-based), searching layout roots, def
 * bodies, then detached icon declarations when a scene key is synthetic. */
export function nodeAtLine(doc: SrcDoc, line: number): SrcTarget | null {
   let hit: SrcTarget | null = null;
   const walk = (list: SrcNode[], def: string | null): void => {
      for (const nd of list) {
         if (hit) return;
         if (nd.line === line) {
            hit = { node: nd, def };
            return;
         }
         walk(nd.children, def);
      }
   };
   walk(doc.roots, null);
   if (!hit) {
      for (const d of doc.defs.values()) {
         walk(d.body, d.name);
         if (hit) break;
      }
   }
   if (!hit) {
      for (const icon of doc.icons) {
         walk([icon.node], null);
         if (hit) break;
      }
   }
   return hit;
}

// ── edits ────────────────────────────────────────────────────────────

/** Set (or add) `name=value` on the node's header. */
export function setAttr(node: SrcNode, name: string, value: string): Change[] {
   const a = node.attrs.find((x) => x.name === name);
   if (a) return [{ from: a.valueSpan.from, to: a.valueSpan.to, insert: value }];
   return [{ from: node.headerSpan.to, to: node.headerSpan.to, insert: ` ${name}=${value}` }];
}

/** Remove an attribute (and the space before it). */
export function removeAttr(doc: SrcDoc, node: SrcNode, name: string): Change[] {
   const a = node.attrs.find((x) => x.name === name);
   if (!a) return [];
   let from = a.nameSpan.from;
   while (from > 0 && (doc.text[from - 1] === ' ' || doc.text[from - 1] === '\t')) from--;
   return [{ from, to: a.valueSpan.to, insert: '' }];
}

/** Toggle a boolean flag on the node's header. */
export function setFlag(doc: SrcDoc, node: SrcNode, name: string, on: boolean): Change[] {
   const f = node.flags.find((x) => x.name === name);
   if (on === (f !== undefined)) return [];
   if (on) return [{ from: node.headerSpan.to, to: node.headerSpan.to, insert: ` ${name}` }];
   if (!f) return [];
   let from = f.span.from;
   while (from > 0 && (doc.text[from - 1] === ' ' || doc.text[from - 1] === '\t')) from--;
   return [{ from, to: f.span.to, insert: '' }];
}

/** Replace the `i`th positional arg (string args keep quoting). */
export function setArg(node: SrcNode, i: number, text: string): Change[] {
   const a = node.args[i];
   if (!a) return [];
   const insert = a.kind === 'string' ? JSON.stringify(text) : text;
   return [{ from: a.span.from, to: a.span.to, insert }];
}

/** Set, add, or clear (`null`) the node's `#id`. */
export function setId(node: SrcNode, id: string | null): Change[] {
   if (node.idSpan) {
      if (id === null || id === '') {
         return [{ from: node.nameSpan.to, to: node.idSpan.to, insert: '' }];
      }
      return [{ from: node.idSpan.from, to: node.idSpan.to, insert: `#${id}` }];
   }
   if (id === null || id === '') return [];
   return [{ from: node.nameSpan.to, to: node.nameSpan.to, insert: `#${id}` }];
}

/** Whole-line extent of a node (for delete/move), including the newline. */
function lineExtent(doc: SrcDoc, node: SrcNode): Span {
   const text = doc.text;
   let from = node.span.from;
   if (node.ownLine) from -= node.indent.length;
   let to = node.span.to;
   while (to < text.length && text[to] !== '\n') {
      // trailing same-line content (comment / `}` of a sibling) keeps the line
      if (!/[ \t;]/.test(text[to])) break;
      to++;
   }
   if (text[to] === '\n') to++;
   return { from, to };
}

/** Delete a node (whole lines when it owns them). */
export function deleteNode(doc: SrcDoc, node: SrcNode): Change[] {
   if (!node.ownLine) return [{ from: node.span.from, to: node.span.to, insert: '' }];
   const ext = lineExtent(doc, node);
   if (doc.text.slice(node.span.to, ext.to).trim() !== '') {
      // something else lives after the node on its closing line — surgical cut
      return [{ from: node.span.from - node.indent.length, to: node.span.to, insert: '' }];
   }
   return [{ from: ext.from, to: ext.to, insert: '' }];
}

/** Re-indent a multi-line snippet from one base indent to another. */
function reindent(snippet: string, fromIndent: string, toIndent: string): string {
   return snippet
      .split('\n')
      .map((l, i) => {
         if (i === 0) return toIndent + l;
         if (l.startsWith(fromIndent)) return toIndent + l.slice(fromIndent.length);
         return l.trim() === '' ? '' : toIndent + l.trimStart();
      })
      .join('\n');
}

export interface InsertPoint {
   /** Text offset the snippet goes to. */
   at: number;
   /** Indent applied to the snippet. */
   indent: string;
   /** Wrapping applied around the snippet ('' when the parent had a block). */
   prefix: string;
   suffix: string;
}

/** Where a child snippet lands inside `parent` at `index` (clamped;
 * `parent === null` appends at document top level). Opens a block on
 * childless parents. */
export function childInsertPoint(doc: SrcDoc, parent: SrcNode | null, index: number): InsertPoint {
   if (parent === null) {
      const at = doc.text.endsWith('\n') ? doc.text.length : doc.text.length;
      const nl = doc.text.length === 0 || doc.text.endsWith('\n') ? '' : '\n';
      return { at, indent: '', prefix: nl, suffix: '\n' };
   }
   const indent = (parent.ownLine ? parent.indent : '') + doc.indentUnit;
   if (parent.bodySpan) {
      const kids = parent.children;
      const i = Math.max(0, Math.min(index, kids.length));
      if (kids.length === 0) {
         return {
            at: parent.bodySpan.from,
            indent,
            prefix: '\n',
            suffix: `\n${parent.ownLine ? parent.indent : ''}`,
         };
      }
      if (i === kids.length) {
         const last = kids[kids.length - 1];
         const ext = lineExtent(doc, last);
         return { at: ext.to, indent, prefix: '', suffix: '\n' };
      }
      const ext = lineExtent(doc, kids[i]);
      return { at: ext.from, indent, prefix: '', suffix: '\n' };
   }
   const pIndent = parent.ownLine ? parent.indent : '';
   return {
      at: parent.headerSpan.to,
      indent,
      prefix: ' {\n',
      suffix: `\n${pIndent}}`,
   };
}

/** A structural edit plus a caret into the POST-change text that lands
 * inside the affected node (reparse + `nodeAtPos(caret)` re-finds it). */
export interface StructEdit {
   changes: Change[];
   caret: number;
}

/** Insert a one-node snippet as a child of `parent` at `index`. */
export function insertChild(
   doc: SrcDoc,
   parent: SrcNode | null,
   snippet: string,
   index: number,
): StructEdit {
   const pt = childInsertPoint(doc, parent, index);
   const body = snippet
      .split('\n')
      .map((l) => pt.indent + l)
      .join('\n');
   return {
      changes: [{ from: pt.at, to: pt.at, insert: `${pt.prefix}${body}${pt.suffix}` }],
      caret: pt.at + pt.prefix.length + pt.indent.length + 1,
   };
}

/** Move a node under `newParent` at `index` (same or different parent).
 * Returns disjoint delete+insert changes against the current text; empty
 * changes when the move is a no-op or illegal (into own subtree). */
export function moveNode(
   doc: SrcDoc,
   node: SrcNode,
   newParent: SrcNode | null,
   index: number,
): StructEdit {
   const noop = { changes: [], caret: node.span.from };
   if (newParent) {
      // refuse moves into the node's own subtree
      for (let a: SrcNode | null = newParent; a; a = a.parent) {
         if (a === node) return noop;
      }
   }
   const sibs = node.parent ? node.parent.children : doc.roots;
   const curIx = sibs.indexOf(node);
   if (newParent === node.parent && (index === curIx || index === curIx + 1)) return noop;

   const ext = lineExtent(doc, node);
   const raw = doc.text.slice(ext.from, ext.to).replace(/\n$/, '');
   const pt = childInsertPoint(doc, newParent, index);
   if (pt.at >= ext.from && pt.at <= ext.to) return noop;
   const snippet = reindent(
      raw.startsWith(node.indent) ? raw.slice(node.indent.length) : raw.trimStart(),
      node.indent,
      '',
   );
   const body = snippet
      .split('\n')
      .map((l) => (l === '' ? '' : pt.indent + l))
      .join('\n');
   // caret in post-change coordinates: the insert shifts by the deletion
   // when the deleted range precedes it
   const shift = ext.from < pt.at ? ext.to - ext.from : 0;
   return {
      changes: [
         { from: ext.from, to: ext.to, insert: '' },
         { from: pt.at, to: pt.at, insert: `${pt.prefix}${body}${pt.suffix}` },
      ],
      caret: pt.at - shift + pt.prefix.length + pt.indent.length + 1,
   };
}

/** Duplicate a node in place (ids stripped — they must stay unique). */
export function duplicateNode(doc: SrcDoc, node: SrcNode): StructEdit {
   const ext = lineExtent(doc, node);
   let raw = doc.text.slice(ext.from, ext.to);
   if (!raw.endsWith('\n')) raw += '\n';
   // strip #ids anywhere in the copy (idSpans of the subtree)
   const cuts: Span[] = [];
   const collect = (nd: SrcNode): void => {
      if (nd.idSpan) cuts.push(nd.idSpan);
      for (const c of nd.children) collect(c);
   };
   collect(node);
   cuts.sort((a, b) => b.from - a.from);
   for (const c of cuts) {
      if (c.from >= ext.from && c.to <= ext.to) {
         raw = raw.slice(0, c.from - ext.from) + raw.slice(c.to - ext.from);
      }
   }
   return {
      changes: [{ from: ext.to, to: ext.to, insert: raw }],
      caret: ext.to + node.indent.length + 1,
   };
}
