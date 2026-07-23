// Node-bindgen WASM conformance runner. It drives the same compiled SLIR bytes
// and byte-compares the same 64 manifest and trace behaviors as the native
// runner. Structured output is formatted by the Rust kernel, never by this host.
//
//   bun tools/conformance-wasm.ts [--selftest]

import { readdir } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { KInst } from '../target/kernel-wasm-node/slab_kernel.js';

const root = dirname(import.meta.dir);
const slirDir = join(root, 'target/conformance-slir');
const CLIENT: Readonly<Record<string, number>> = { web: 0, gpu: 1, tui: 2, svg: 3, png: 4 };
const PARAM_KIND: Readonly<Record<string, number>> = {
   text: 0,
   num: 1,
   pct: 2,
   color: 3,
   bool: 4,
   enum: 5,
};
const EVENT_CODE: Readonly<Record<string, number>> = {
   'pointer-move': 0,
   'pointer-down': 1,
   'pointer-up': 2,
   wheel: 3,
   'key-down': 4,
   text: 5,
   paste: 6,
   copy: 7,
   cut: 8,
   'composition-start': 9,
   'composition-update': 10,
   'composition-end': 11,
   blur: 12,
   resize: 13,
   close: 14,
   inspect: 15,
   activate: 16,
};
const MOD_BIT: Readonly<Record<string, number>> = { shift: 1, alt: 2, ctrl: 4, meta: 8 };

interface ManifestCase {
   name: string;
   source?: string;
   kind?: string;
   width?: number;
   height?: number;
   client?: string;
   states?: string[];
   statesPrev?: string[];
   stateAge?: number;
   theme?: string;
   set?: unknown;
   time?: number;
}

interface ParamInput {
   name: string;
   kind: string;
   value: unknown;
}

interface TraceState {
   key?: string;
   name: string;
   on?: boolean;
}

interface TraceEnv {
   vw?: number;
   vh?: number;
   client?: string;
   dark?: boolean;
   coarse?: boolean;
}

interface TraceScroll {
   key: string;
   axis?: number;
   off: number;
}

interface TraceImageRegister {
   op: 'register';
   name: string;
   w: number;
   h: number;
   format: number;
   bytes: Uint8Array;
}

interface TraceImageUnregister {
   op: 'unregister';
   name: string;
}

type TraceImage = TraceImageRegister | TraceImageUnregister;

interface TraceList {
   param: string;
   path: string;
   op: string;
   n?: number;
   index?: number;
   field?: string;
   key?: string;
   kind?: string;
   value: unknown;
}

interface TraceDivider {
   key: string;
   extent: number;
}

interface TraceReveal {
   key: string;
   margin: number;
}

interface TraceRevealItem {
   each: string;
   index: number;
   align: number;
}

interface TraceWindow {
   each: string;
}

interface TraceFocus {
   key: string;
   visible?: boolean;
}

interface TraceHole {
   hole: number;
   w: number;
   h: number;
}

interface TraceStep {
   time?: number;
   event?: object;
   state?: TraceState;
   env?: TraceEnv;
   param?: ParamInput;
   scroll?: TraceScroll;
   focus?: TraceFocus;
   hole?: TraceHole;
   img?: TraceImage;
   list?: TraceList;
   divider?: TraceDivider;
   reveal?: TraceReveal;
   revealItem?: TraceRevealItem;
   window?: TraceWindow;
   hit?: [number, number];
}

interface ExpectedSignal {
   name: string;
   text: string;
   item: string;
}

interface ExpectedEdit {
   name: string;
   text: string;
}

interface ExpectedScroll {
   key: string;
   axis: number;
   off: number;
}

interface TraceExpect {
   signals?: ExpectedSignal[];
   focusKey?: unknown;
   edits?: ExpectedEdit[];
   scroll?: ExpectedScroll[];
}

interface TraceCase {
   doc: string;
   env: TraceEnv;
   params: ParamInput[];
   steps: TraceStep[];
   expect?: TraceExpect;
}

interface ParamDef {
   name: string;
   ty: number;
   enumSymbols: string[];
}

interface ParamValue {
   kind: number;
   num: number;
   value: string;
   rgba: number;
   symbol: string;
}

interface ListFieldDef {
   name: string;
   ty: number;
   defaultValue: ParamValue;
   enumSymbols: string[];
}

interface ListDef {
   param: number;
   fields: ListFieldDef[];
}

interface Statics {
   params: ParamDef[];
   lists: ListDef[];
}

interface GoldenOutput {
   name: string;
   payload: string;
}

interface Signal {
   name: string;
   text: string;
   item: string;
}

interface RuntimeEnv {
   vw: number;
   vh: number;
   client: number;
   dark: boolean;
   coarse: boolean;
}

type PreparedSet =
   | { tag: 'scalar'; param: number; value: ParamValue }
   | { tag: 'list'; param: number; items: { key: string; fields: [string, ParamValue][] }[] };

function failShape(context: string): never {
   throw new Error(`${context} has an invalid shape`);
}

function objectValue(value: unknown, context: string): object {
   if (value === null || typeof value !== 'object' || Array.isArray(value)) failShape(context);
   return value;
}

function field(value: object, name: string): unknown {
   return Reflect.get(value, name);
}

function requiredString(value: unknown, context: string): string {
   if (typeof value !== 'string') failShape(context);
   return value;
}

function optionalString(value: unknown, context: string): string | undefined {
   if (value === undefined) return undefined;
   return requiredString(value, context);
}

function requiredNumber(value: unknown, context: string): number {
   if (typeof value !== 'number' || !Number.isFinite(value)) failShape(context);
   return value;
}

function optionalNumber(value: unknown, context: string): number | undefined {
   if (value === undefined) return undefined;
   return requiredNumber(value, context);
}

function optionalBoolean(value: unknown, context: string): boolean | undefined {
   if (value === undefined) return undefined;
   if (typeof value !== 'boolean') failShape(context);
   return value;
}

function stringArray(value: unknown, context: string): string[] {
   if (!Array.isArray(value)) failShape(context);
   return value.map((item, index) => requiredString(item, `${context}[${index}]`));
}

function optionalStringArray(value: unknown, context: string): string[] | undefined {
   if (value === undefined) return undefined;
   return stringArray(value, context);
}

function stringOr(value: unknown, fallback: string): string {
   return typeof value === 'string' ? value : fallback;
}

function b64Digit(byte: number): number | undefined {
   if (byte >= 65 && byte <= 90) return byte - 65;
   if (byte >= 97 && byte <= 122) return byte - 97 + 26;
   if (byte >= 48 && byte <= 57) return byte - 48 + 52;
   if (byte === 43) return 62;
   if (byte === 47) return 63;
   return undefined;
}

function decodeB64(input: string): Uint8Array {
   const bytes = new TextEncoder().encode(input);
   if (bytes.length % 4 !== 0) {
      throw new Error('base64 payload length must be a multiple of four');
   }
   const output = new Uint8Array((bytes.length / 4) * 3);
   let length = 0;
   for (let offset = 0; offset < bytes.length; offset += 4) {
      const finalChunk = offset + 4 === bytes.length;
      const a = b64Digit(bytes[offset]);
      const b = b64Digit(bytes[offset + 1]);
      if (a === undefined || b === undefined) throw new Error('invalid base64 payload');
      output[length++] = (a << 2) | (b >> 4);
      const rawC = bytes[offset + 2];
      const rawD = bytes[offset + 3];
      if (rawC === 61 && rawD === 61 && finalChunk && (b & 0x0f) === 0) continue;
      const c = b64Digit(rawC);
      if (c === undefined) throw new Error('invalid base64 payload');
      if (rawD === 61 && finalChunk) {
         if ((c & 0x03) !== 0) throw new Error('invalid base64 payload');
         output[length++] = (b << 4) | (c >> 2);
         continue;
      }
      const d = b64Digit(rawD);
      if (d === undefined) throw new Error('invalid base64 payload');
      output[length++] = (b << 4) | (c >> 2);
      output[length++] = (c << 6) | d;
   }
   return output.subarray(0, length);
}

function u32(value: unknown, message: string): number {
   if (typeof value !== 'number' || !Number.isInteger(value) || value < 0 || value > 0xffff_ffff) {
      throw new Error(message);
   }
   return value;
}

function runtimeImageInput(value: object): TraceImageRegister {
   const name = field(value, 'name');
   if (typeof name !== 'string') throw new Error("image 'name' must be a string");
   const w = u32(field(value, 'w'), "image 'w' must be a u32");
   const h = u32(field(value, 'h'), "image 'h' must be a u32");
   const format = u32(field(value, 'format'), "image 'format' must be a u32");
   const rgba = field(value, 'rgba');
   const png = field(value, 'png_b64');
   let bytes: Uint8Array;
   if (rgba !== undefined && png !== undefined) {
      throw new Error("image needs exactly one of 'rgba' or 'png_b64'");
   }
   if (rgba !== undefined) {
      if (format !== 1) throw new Error("image 'rgba' requires format 1");
      if (!Array.isArray(rgba)) throw new Error("image 'rgba' must be a byte array");
      bytes = new Uint8Array(rgba.length);
      for (let index = 0; index < rgba.length; index++) {
         const byte = rgba[index];
         if (typeof byte !== 'number' || !Number.isInteger(byte) || byte < 0 || byte > 255) {
            throw new Error("image 'rgba' must contain only bytes");
         }
         bytes[index] = byte;
      }
   } else if (png !== undefined) {
      if (format !== 0) throw new Error("image 'png_b64' requires format 0");
      if (typeof png !== 'string') throw new Error("image 'png_b64' must be a string");
      bytes = decodeB64(png);
   } else {
      throw new Error("image needs exactly one of 'rgba' or 'png_b64'");
   }
   return { op: 'register', name, w, h, format, bytes };
}

async function readJson(path: string): Promise<unknown> {
   return JSON.parse(await Bun.file(path).text());
}

function parseManifestCase(value: unknown, index: number): ManifestCase {
   const item = objectValue(value, `manifest case ${index}`);
   return {
      name: requiredString(field(item, 'name'), `manifest case ${index}.name`),
      source: optionalString(field(item, 'source'), `manifest case ${index}.source`),
      kind: optionalString(field(item, 'kind'), `manifest case ${index}.kind`),
      width: optionalNumber(field(item, 'width'), `manifest case ${index}.width`),
      height: optionalNumber(field(item, 'height'), `manifest case ${index}.height`),
      client: optionalString(field(item, 'client'), `manifest case ${index}.client`),
      states: optionalStringArray(field(item, 'states'), `manifest case ${index}.states`),
      statesPrev: optionalStringArray(
         field(item, 'states_prev'),
         `manifest case ${index}.states_prev`,
      ),
      stateAge: optionalNumber(field(item, 'state_age'), `manifest case ${index}.state_age`),
      theme: optionalString(field(item, 'theme'), `manifest case ${index}.theme`),
      set: field(item, 'set'),
      time: optionalNumber(field(item, 't'), `manifest case ${index}.t`),
   };
}

async function readManifest(): Promise<ManifestCase[]> {
   const manifest = objectValue(
      await readJson(join(root, 'conformance/manifest.json')),
      'conformance manifest',
   );
   const cases = field(manifest, 'cases');
   if (!Array.isArray(cases)) failShape('conformance manifest cases');
   return cases.map(parseManifestCase);
}

function parseTraceEnv(value: unknown, context: string): TraceEnv {
   const env = objectValue(value, context);
   return {
      vw: optionalNumber(field(env, 'vw'), `${context}.vw`),
      vh: optionalNumber(field(env, 'vh'), `${context}.vh`),
      client: optionalString(field(env, 'client'), `${context}.client`),
      dark: optionalBoolean(field(env, 'dark'), `${context}.dark`),
      coarse: optionalBoolean(field(env, 'coarse'), `${context}.coarse`),
   };
}

function parseParam(value: unknown, context: string): ParamInput {
   const param = objectValue(value, context);
   return {
      name: requiredString(field(param, 'name'), `${context}.name`),
      kind: requiredString(field(param, 'kind'), `${context}.kind`),
      value: field(param, 'value'),
   };
}

function parseTraceStep(value: unknown, index: number): TraceStep {
   const context = `trace step ${index}`;
   const step = objectValue(value, context);
   const stateValue = field(step, 'state');
   let state: TraceState | undefined;
   if (stateValue !== undefined) {
      const stateObject = objectValue(stateValue, `${context}.state`);
      state = {
         key: optionalString(field(stateObject, 'key'), `${context}.state.key`),
         name: requiredString(field(stateObject, 'name'), `${context}.state.name`),
         on: optionalBoolean(field(stateObject, 'on'), `${context}.state.on`),
      };
   }
   const scrollValue = field(step, 'scroll');
   let scroll: TraceScroll | undefined;
   if (scrollValue !== undefined) {
      const scrollObject = objectValue(scrollValue, `${context}.scroll`);
      const rawAxis = field(scrollObject, 'axis');
      let axis: number | undefined;
      if (rawAxis === undefined) {
         axis = undefined;
      } else if (typeof rawAxis === 'number' && Number.isFinite(rawAxis)) {
         axis = rawAxis;
      } else {
         axis = Number.NaN;
      }
      scroll = {
         key: requiredString(field(scrollObject, 'key'), `${context}.scroll.key`),
         axis,
         off: requiredNumber(field(scrollObject, 'off'), `${context}.scroll.off`),
      };
   }
   const focusValue = field(step, 'focus');
   let focus: TraceFocus | undefined;
   if (focusValue !== undefined) {
      const focusObject = objectValue(focusValue, `${context}.focus`);
      focus = {
         key: requiredString(field(focusObject, 'key'), `${context}.focus.key`),
         visible: optionalBoolean(field(focusObject, 'visible'), `${context}.focus.visible`),
      };
   }
   const holeValue = field(step, 'hole');
   let hole: TraceHole | undefined;
   if (holeValue !== undefined) {
      const holeObject = objectValue(holeValue, `${context}.hole`);
      hole = {
         hole: u32Or(field(holeObject, 'hole'), 0xffff_ffff),
         w: requiredNumber(field(holeObject, 'w'), `${context}.hole.w`),
         h: requiredNumber(field(holeObject, 'h'), `${context}.hole.h`),
      };
   }
   const imgValue = field(step, 'img');
   let img: TraceImage | undefined;
   if (imgValue !== undefined) {
      const imageObject = objectValue(imgValue, `${context}.img`);
      const op = field(imageObject, 'op');
      if (op === undefined) {
         img = runtimeImageInput(imageObject);
      } else if (op === 'unregister') {
         img = { op: 'unregister', name: stringOr(field(imageObject, 'name'), '') };
      } else {
         throw new Error(`unknown img op '${stringOr(op, '')}'`);
      }
   }
   const listValue = field(step, 'list');
   let list: TraceList | undefined;
   if (listValue !== undefined) {
      const listObject = objectValue(listValue, `${context}.list`);
      list = {
         param: stringOr(field(listObject, 'param'), ''),
         path: stringOr(field(listObject, 'path'), ''),
         op: stringOr(field(listObject, 'op'), ''),
         n: i32Or(field(listObject, 'n'), -0x8000_0000),
         index: i32Or(field(listObject, 'index'), -0x8000_0000),
         field: stringOr(field(listObject, 'field'), ''),
         key: stringOr(field(listObject, 'key'), ''),
         kind: stringOr(field(listObject, 'kind'), ''),
         value: field(listObject, 'value'),
      };
   }
   const dividerValue = field(step, 'divider');
   let divider: TraceDivider | undefined;
   if (dividerValue !== undefined) {
      const dividerObject = objectValue(dividerValue, `${context}.divider`);
      const rawExtent = field(dividerObject, 'extent');
      divider = {
         key: stringOr(field(dividerObject, 'key'), ''),
         extent:
            typeof rawExtent === 'number' && Number.isFinite(rawExtent) ? rawExtent : Number.NaN,
      };
   }
   const revealValue = field(step, 'reveal');
   let reveal: TraceReveal | undefined;
   if (revealValue !== undefined) {
      const revealObject = objectValue(revealValue, `${context}.reveal`);
      const rawMargin = field(revealObject, 'margin');
      reveal = {
         key: stringOr(field(revealObject, 'key'), ''),
         margin: typeof rawMargin === 'number' && Number.isFinite(rawMargin) ? rawMargin : 0,
      };
   }
   const revealItemValue = field(step, 'reveal_item');
   let revealItem: TraceRevealItem | undefined;
   if (revealItemValue !== undefined) {
      const revealItemObject = objectValue(revealItemValue, `${context}.reveal_item`);
      revealItem = {
         each: stringOr(field(revealItemObject, 'each'), ''),
         index: i32Or(field(revealItemObject, 'index'), -0x8000_0000),
         align: u32Or(field(revealItemObject, 'align'), 0xffff_ffff),
      };
   }
   const windowValue = field(step, 'window');
   let window: TraceWindow | undefined;
   if (windowValue !== undefined) {
      const windowObject = objectValue(windowValue, `${context}.window`);
      window = {
         each: stringOr(field(windowObject, 'each'), ''),
      };
   }
   const hitValue = field(step, 'hit');
   let hit: [number, number] | undefined;
   if (hitValue !== undefined) {
      if (!Array.isArray(hitValue) || hitValue.length !== 2) failShape(`${context}.hit`);
      hit = [
         requiredNumber(hitValue[0], `${context}.hit[0]`),
         requiredNumber(hitValue[1], `${context}.hit[1]`),
      ];
   }
   const eventValue = field(step, 'event');
   const envValue = field(step, 'env');
   const paramValue = field(step, 'param');
   return {
      time: optionalNumber(field(step, 't'), `${context}.t`),
      event: eventValue === undefined ? undefined : objectValue(eventValue, `${context}.event`),
      state,
      env: envValue === undefined ? undefined : parseTraceEnv(envValue, `${context}.env`),
      param: paramValue === undefined ? undefined : parseParam(paramValue, `${context}.param`),
      scroll,
      focus,
      hole,
      img,
      list,
      divider,
      reveal,
      revealItem,
      window,
      hit,
   };
}

function parseTraceExpect(value: unknown): TraceExpect | undefined {
   if (value === null || typeof value !== 'object' || Array.isArray(value)) return undefined;
   const expect = value;
   const signalsValue = field(expect, 'signals');
   const signals = Array.isArray(signalsValue)
      ? signalsValue.map((item) => {
           const signal =
              item !== null && typeof item === 'object' && !Array.isArray(item) ? item : {};
           return {
              name: stringOr(field(signal, 'name'), ''),
              text: stringOr(field(signal, 'text'), ''),
              item: stringOr(field(signal, 'item'), ''),
           };
        })
      : undefined;
   const editsValue = field(expect, 'edits');
   const edits = Array.isArray(editsValue)
      ? editsValue.map((item) => {
           const edit =
              item !== null && typeof item === 'object' && !Array.isArray(item) ? item : {};
           return {
              name: stringOr(field(edit, 'name'), ''),
              text: stringOr(field(edit, 'text'), ''),
           };
        })
      : undefined;
   const scrollValue = field(expect, 'scroll');
   const scroll = Array.isArray(scrollValue)
      ? scrollValue.map((item) => {
           const entry =
              item !== null && typeof item === 'object' && !Array.isArray(item) ? item : {};
           const rawOff = field(entry, 'off');
           return {
              key: stringOr(field(entry, 'key'), ''),
              axis: u32Or(field(entry, 'axis'), 0),
              off: typeof rawOff === 'number' && Number.isFinite(rawOff) ? rawOff : 0,
           };
        })
      : undefined;
   return {
      signals,
      focusKey: field(expect, 'focus_key'),
      edits,
      scroll,
   };
}

function parseTraceCase(value: unknown): TraceCase {
   const trace = objectValue(value, 'trace case');
   const paramsValue = field(trace, 'params');
   const stepsValue = field(trace, 'steps');
   if (paramsValue !== undefined && !Array.isArray(paramsValue)) failShape('trace params');
   if (!Array.isArray(stepsValue)) failShape('trace steps');
   return {
      doc: requiredString(field(trace, 'doc'), 'trace doc'),
      env: parseTraceEnv(field(trace, 'env'), 'trace env'),
      params: paramsValue?.map((param, index) => parseParam(param, `trace param ${index}`)) ?? [],
      steps: stepsValue.map(parseTraceStep),
      expect: parseTraceExpect(field(trace, 'expect')),
   };
}

function parseParamValue(value: unknown, context: string): ParamValue {
   const snapshot = objectValue(value, context);
   return {
      kind: requiredNumber(field(snapshot, 'kind'), `${context}.kind`),
      num: requiredNumber(field(snapshot, 'num'), `${context}.num`),
      value: requiredString(field(snapshot, 's'), `${context}.s`),
      rgba: requiredNumber(field(snapshot, 'rgba'), `${context}.rgba`) >>> 0,
      symbol: requiredString(field(snapshot, 'sym'), `${context}.sym`),
   };
}

function parseStatics(json: string): Statics {
   const parsed: unknown = JSON.parse(json);
   const statics = objectValue(parsed, 'statics');
   const paramsValue = field(statics, 'params');
   const listsValue = field(statics, 'lists');
   if (!Array.isArray(paramsValue) || !Array.isArray(listsValue)) failShape('statics schemas');
   const params = paramsValue.map((value, index) => {
      const param = objectValue(value, `statics.params[${index}]`);
      return {
         name: requiredString(field(param, 'name'), `statics.params[${index}].name`),
         ty: requiredNumber(field(param, 'ty'), `statics.params[${index}].ty`),
         enumSymbols: stringArray(
            field(param, 'enum_symbols'),
            `statics.params[${index}].enum_symbols`,
         ),
      };
   });
   const lists = listsValue.map((value, index) => {
      const list = objectValue(value, `statics.lists[${index}]`);
      const fieldsValue = field(list, 'fields');
      if (!Array.isArray(fieldsValue)) failShape(`statics.lists[${index}].fields`);
      return {
         param: requiredNumber(field(list, 'param'), `statics.lists[${index}].param`),
         fields: fieldsValue.map((value, fieldIndex) => {
            const context = `statics.lists[${index}].fields[${fieldIndex}]`;
            const listField = objectValue(value, context);
            return {
               name: requiredString(field(listField, 'name'), `${context}.name`),
               ty: requiredNumber(field(listField, 'ty'), `${context}.ty`),
               defaultValue: parseParamValue(field(listField, 'default'), `${context}.default`),
               enumSymbols: stringArray(
                  field(listField, 'enum_symbols'),
                  `${context}.enum_symbols`,
               ),
            };
         }),
      };
   });
   return { params, lists };
}

async function ensureSlir(name: string, source = name): Promise<Uint8Array> {
   const path = join(slirDir, `${name}.slir`);
   if (!(await Bun.file(path).exists())) {
      const src = join(root, 'conformance/cases', `${source}.slab`);
      const out = await Bun.$`cargo run -q -p slab-cli -- build ${src} -o ${path}`
         .cwd(root)
         .quiet()
         .nothrow();
      if (out.exitCode !== 0) throw new Error(`compile ${name}: ${out.stderr.toString()}`);
   }
   return new Uint8Array(await Bun.file(path).arrayBuffer());
}

async function ensureTraceSlir(doc: string): Promise<Uint8Array> {
   const path = join(slirDir, 'traces', `${doc}.slir`);
   if (!(await Bun.file(path).exists())) {
      const src = join(root, 'conformance/cases', `${doc}.slab`);
      const out = await Bun.$`cargo run -q -p slab-cli -- build ${src} -o ${path}`
         .cwd(root)
         .quiet()
         .nothrow();
      if (out.exitCode !== 0) throw new Error(`compile ${doc}: ${out.stderr.toString()}`);
   }
   return new Uint8Array(await Bun.file(path).arrayBuffer());
}

function clientCode(name: string): number {
   const client = CLIENT[name];
   if (client === undefined) throw new Error(`unknown client '${name}'`);
   return client;
}

function emptyParam(kind: number): ParamValue {
   return { kind, num: 0, value: '', rgba: 0, symbol: '' };
}

function traceParamValue(param: ParamInput): ParamValue {
   const kind = PARAM_KIND[param.kind];
   if (kind === undefined) throw new Error(`unknown param kind '${param.kind}'`);
   const value = emptyParam(kind);
   if (kind === 0) {
      if (typeof param.value !== 'string') throw new Error('text value must be a string');
      value.value = param.value;
   } else if (kind === 1 || kind === 2) {
      if (typeof param.value !== 'number' || !Number.isFinite(param.value)) {
         throw new Error('numeric value must be a number');
      }
      value.num = param.value;
   } else if (kind === 3) {
      value.rgba = u32(param.value, 'color value must be a u32');
   } else if (kind === 4) {
      if (typeof param.value === 'boolean') {
         value.num = param.value ? 1 : 0;
      } else if (typeof param.value === 'number' && Number.isFinite(param.value)) {
         value.num = param.value;
      } else {
         throw new Error('bool value must be a number or boolean');
      }
   } else {
      if (typeof param.value !== 'string') throw new Error('enum value must be a string');
      value.symbol = param.value;
   }
   return value;
}

function setParam(inst: KInst, param: number, value: ParamValue): boolean {
   return inst.set_param(param, value.kind, value.num, value.value, value.rgba, value.symbol);
}

function setListField(
   inst: KInst,
   param: number,
   path: string,
   index: number,
   name: string,
   value: ParamValue,
): boolean {
   return inst.set_list_field(
      param,
      path,
      index,
      name,
      value.kind,
      value.num,
      value.value,
      value.rgba,
      value.symbol,
   );
}

function paramIndex(statics: Statics, name: string): number {
   return statics.params.findIndex((param) => param.name === name);
}

function i32Or(value: unknown, fallback: number): number {
   return typeof value === 'number' &&
      Number.isInteger(value) &&
      value >= -0x8000_0000 &&
      value <= 0x7fff_ffff
      ? value
      : fallback;
}

function u32Or(value: unknown, fallback: number): number {
   return typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 0xffff_ffff
      ? value
      : fallback;
}

function finiteNumber(raw: string, what: string): number {
   if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/.test(raw)) {
      throw new Error(`'${raw}' is not ${what}`);
   }
   const value = Number(raw);
   if (!Number.isFinite(value)) throw new Error(`'${raw}' is not ${what}`);
   return value;
}

function parseSlabColor(raw: string): number | null {
   const source = raw.trim();
   let rgba: number[] | null = null;
   if (source === 'white') rgba = [255, 255, 255, 255];
   else if (source === 'black') rgba = [0, 0, 0, 255];
   else if (source.startsWith('#')) {
      let hex = source.slice(1);
      if (hex.length === 3 || hex.length === 4) {
         hex = [...hex].map((digit) => digit + digit).join('');
      }
      if (hex.length === 6) hex += 'ff';
      if (/^[0-9a-fA-F]{8}$/.test(hex)) {
         rgba = [
            Number.parseInt(hex.slice(0, 2), 16),
            Number.parseInt(hex.slice(2, 4), 16),
            Number.parseInt(hex.slice(4, 6), 16),
            Number.parseInt(hex.slice(6, 8), 16),
         ];
      }
   } else {
      const functional = /^(rgba?|oklch)\((.*)\)$/.exec(source);
      if (functional) {
         const parts = functional[2].split(/[,/\s]+/).filter(Boolean);
         const percent = (part: string, scale: number) =>
            finiteNumber(part.endsWith('%') ? part.slice(0, -1) : part, 'a color component') *
            (part.endsWith('%') ? scale : 1);
         if ((functional[1] === 'rgb' || functional[1] === 'rgba') && parts.length >= 3) {
            const channels = parts
               .slice(0, 3)
               .map((part) => Math.trunc(Math.max(0, Math.min(255, percent(part, 2.55)))));
            const alpha =
               parts[3] === undefined
                  ? 255
                  : Math.round(Math.max(0, Math.min(255, percent(parts[3], 2.55))));
            if (!parts[3]?.endsWith('%') && parts[3] !== undefined) {
               channels.push(
                  Math.round(Math.max(0, Math.min(255, finiteNumber(parts[3], 'an alpha') * 255))),
               );
            } else {
               channels.push(alpha);
            }
            rgba = channels;
         } else if (functional[1] === 'oklch' && parts.length >= 3) {
            const lightness = parts[0].endsWith('%')
               ? finiteNumber(parts[0].slice(0, -1), 'a lightness') / 100
               : finiteNumber(parts[0], 'a lightness');
            const chroma = finiteNumber(parts[1], 'a chroma');
            const radians = (finiteNumber(parts[2], 'a hue') * Math.PI) / 180;
            const a = chroma * Math.cos(radians);
            const b = chroma * Math.sin(radians);
            const ll = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3;
            const mm = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3;
            const ss = (lightness - 0.0894841775 * a - 1.291485548 * b) ** 3;
            const linear = [
               4.0767416621 * ll - 3.3077115913 * mm + 0.2309699292 * ss,
               -1.2684380046 * ll + 2.6097574011 * mm - 0.3413193965 * ss,
               -0.0041960863 * ll - 0.7034186147 * mm + 1.707614701 * ss,
            ];
            const rgb = linear.map((component) => {
               const x = Math.max(0, component);
               const srgb = x <= 0.0031308 ? 12.92 * x : 1.055 * x ** (1 / 2.4) - 0.055;
               return Math.round(Math.max(0, Math.min(255, srgb * 255)));
            });
            const alpha =
               parts[3] === undefined
                  ? 255
                  : parts[3].endsWith('%')
                    ? Math.round(Math.max(0, Math.min(255, percent(parts[3], 2.55))))
                    : Math.round(
                         Math.max(0, Math.min(255, finiteNumber(parts[3], 'an alpha') * 255)),
                      );
            rgba = [...rgb, alpha];
         }
      }
   }
   if (!rgba) return null;
   return (rgba[0] | (rgba[1] << 8) | (rgba[2] << 16) | (rgba[3] << 24)) >>> 0;
}

function coerceScalar(param: ParamDef, raw: string): ParamValue {
   const value = emptyParam(param.ty);
   if (param.ty === 0) value.value = raw;
   else if (param.ty === 1) value.num = finiteNumber(raw, 'a number');
   else if (param.ty === 2) {
      value.num = finiteNumber(raw.endsWith('%') ? raw.slice(0, -1) : raw, 'a percentage');
   } else if (param.ty === 3) {
      const rgba = parseSlabColor(raw);
      if (rgba === null) throw new Error(`'${raw}' is not a color`);
      value.rgba = rgba;
   } else if (param.ty === 4) {
      if (raw === 'true' || raw === '1' || raw === 'on') value.num = 1;
      else if (raw === 'false' || raw === '0' || raw === 'off') value.num = 0;
      else throw new Error(`'${raw}' is not a bool`);
   } else if (param.ty === 5) {
      if (!param.enumSymbols.includes(raw)) throw new Error(`unknown enum member '${raw}'`);
      value.symbol = raw;
   } else {
      throw new Error(`unsupported param type ${param.ty}`);
   }
   return value;
}

function coerceListField(fieldDef: ListFieldDef, raw: unknown): ParamValue {
   const value = emptyParam(fieldDef.ty);
   if (fieldDef.ty === 0) {
      if (typeof raw !== 'string') throw new Error('must be a string');
      value.value = raw;
   } else if (fieldDef.ty === 1 || fieldDef.ty === 2) {
      if (typeof raw !== 'number' || !Number.isFinite(raw)) {
         throw new Error(fieldDef.ty === 1 ? 'must be a number' : 'must be a percentage number');
      }
      value.num = raw;
   } else if (fieldDef.ty === 3) {
      if (typeof raw !== 'string') throw new Error('must be a color string');
      const rgba = parseSlabColor(raw);
      if (rgba === null) throw new Error(`'${raw}' is not a color`);
      value.rgba = rgba;
   } else if (fieldDef.ty === 4) {
      if (typeof raw !== 'boolean') throw new Error('must be a boolean');
      value.num = raw ? 1 : 0;
   } else if (fieldDef.ty === 5) {
      if (typeof raw !== 'string') throw new Error('must be an enum string');
      if (!fieldDef.enumSymbols.includes(raw)) throw new Error(`unknown enum member '${raw}'`);
      value.symbol = raw;
   } else {
      throw new Error(`unsupported field type ${fieldDef.ty}`);
   }
   return value;
}

function setEntries(set: unknown): [string, unknown][] {
   if (set === undefined || set === null) return [];
   if (Array.isArray(set)) {
      return set.map((entry, index) => {
         if (!Array.isArray(entry) || entry.length !== 2 || typeof entry[0] !== 'string') {
            throw new Error(`sets entry ${index} must be [name, value]`);
         }
         return [entry[0], entry[1]];
      });
   }
   const object = objectValue(set, 'manifest set');
   const entries: [string, unknown][] = [];
   for (const key of Reflect.ownKeys(object)) {
      if (typeof key === 'string') entries.push([key, Reflect.get(object, key)]);
   }
   entries.sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0));
   return entries;
}

function shownJson(value: unknown): string {
   if (typeof value === 'string') return value;
   const encoded: unknown = JSON.stringify(value);
   return typeof encoded === 'string' ? encoded : 'null';
}

function prepareManifestSet(statics: Statics, name: string, input: unknown): PreparedSet {
   const param = paramIndex(statics, name);
   if (param < 0) throw new Error(`unknown param '${name}'`);
   const definition = statics.params[param];
   if (definition.ty !== 6) {
      return { tag: 'scalar', param, value: coerceScalar(definition, shownJson(input)) };
   }
   let parsed = input;
   if (typeof parsed === 'string') {
      try {
         parsed = JSON.parse(parsed);
      } catch (error) {
         throw new Error(`invalid list JSON: ${error}`);
      }
   }
   if (!Array.isArray(parsed)) throw new Error('list value must be a JSON array');
   const schema = statics.lists.find((list) => list.param === param);
   if (!schema) throw new Error('list schema is missing');
   const known = new Set(schema.fields.map((listField) => listField.name));
   const keys = new Set<string>();
   const items = parsed.map((rawItem, index) => {
      const item = objectValue(rawItem, `item ${index}`);
      for (const key of Reflect.ownKeys(item)) {
         if (typeof key === 'string' && key !== 'key' && !known.has(key)) {
            throw new Error(`item ${index}: unknown field '${key}'`);
         }
      }
      const keyValue: unknown = Reflect.get(item, 'key');
      if (keyValue !== undefined && typeof keyValue !== 'string') {
         throw new Error(`item ${index} field 'key' must be a string`);
      }
      const key = keyValue ?? String(index);
      if (keys.has(key)) throw new Error(`item ${index}: duplicate key '${key}'`);
      keys.add(key);
      const fields: [string, ParamValue][] = schema.fields.map((listField) => {
         try {
            return [
               listField.name,
               Object.hasOwn(item, listField.name)
                  ? coerceListField(listField, Reflect.get(item, listField.name))
                  : listField.defaultValue,
            ];
         } catch (error) {
            throw new Error(`item ${index} field '${listField.name}': ${error}`);
         }
      });
      return { key, fields };
   });
   return { tag: 'list', param, items };
}

function applyManifestSet(inst: KInst, statics: Statics, set: unknown): void {
   const prepared = setEntries(set).map(([name, raw]) => {
      try {
         return prepareManifestSet(statics, name, raw);
      } catch (error) {
         throw new Error(`--set ${name}=${shownJson(raw)}: ${error}`);
      }
   });
   for (const entry of prepared) {
      if (entry.tag === 'scalar') {
         if (!setParam(inst, entry.param, entry.value)) {
            throw new Error('validated scalar input was rejected by the kernel');
         }
         continue;
      }
      if (!inst.set_list_len(entry.param, '', entry.items.length)) {
         throw new Error('validated list length was rejected by the kernel');
      }
      for (let index = 0; index < entry.items.length; index++) {
         const item = entry.items[index];
         if (!inst.set_list_key(entry.param, '', index, item.key)) {
            throw new Error('validated list key was rejected by the kernel');
         }
         for (const [name, value] of item.fields) {
            if (!setListField(inst, entry.param, '', index, name, value)) {
               throw new Error('validated list field was rejected by the kernel');
            }
         }
      }
   }
}

function solve(inst: KInst, time: number): void {
   const frame = inst.frame(time);
   try {
      // Solving is the side effect; the binary payload is intentionally unused here.
   } finally {
      frame.free();
   }
}

function setupCase(
   inst: KInst,
   statics: Statics,
   testCase: ManifestCase,
): { time: number; client: number } {
   const client = clientCode(testCase.client ?? 'svg');
   inst.set_env(testCase.width ?? 800, testCase.height ?? 0, client, false, false);
   const theme = testCase.theme ?? '';
   if (!inst.set_theme(theme)) throw new Error(`unknown theme '${theme}'`);
   applyManifestSet(inst, statics, testCase.set);
   let time = testCase.time ?? 0;
   if (testCase.stateAge !== undefined) {
      for (const state of testCase.statesPrev ?? []) inst.set_state(state, true);
      solve(inst, 0);
      for (const state of testCase.statesPrev ?? []) {
         if (!(testCase.states ?? []).includes(state)) inst.set_state(state, false);
      }
      for (const state of testCase.states ?? []) inst.set_state(state, true);
      solve(inst, 0);
      time = testCase.stateAge;
   } else {
      for (const state of testCase.states ?? []) inst.set_state(state, true);
   }
   return { time, client };
}

function renderCase(
   bytes: Uint8Array,
   testCase: ManifestCase,
   render: (inst: KInst, time: number, client: number) => string,
): string {
   const inst = new KInst(bytes);
   try {
      const statics = parseStatics(inst.statics_json());
      const setup = setupCase(inst, statics, testCase);
      return render(inst, setup.time, setup.client);
   } finally {
      inst.free();
   }
}

function runManifestCase(bytes: Uint8Array, testCase: ManifestCase): GoldenOutput[] {
   if (testCase.kind === 'caps') {
      return [
         {
            name: `${testCase.name}.caps.txt`,
            payload: renderCase(bytes, testCase, (inst, time, client) =>
               inst.caps_report(time, client),
            ),
         },
      ];
   }
   const outputs = [
      {
         name: `${testCase.name}.frame.json`,
         payload: `${renderCase(bytes, testCase, (inst, time) => inst.frame_json(time))}\n`,
      },
   ];
   if (testCase.kind === 'tui') {
      outputs.push({
         name: `${testCase.name}.cells.txt`,
         payload: renderCase(bytes, testCase, (inst, time) => inst.cells_text(time)),
      });
   }
   return outputs;
}

function eventArgs(
   event: object,
): [number, number, number, number, number, number, string, string, number, number] {
   const type = String(field(event, 'type') ?? '');
   const eventType = EVENT_CODE[type];
   if (eventType === undefined) throw new Error(`unknown event type '${type}'`);
   const modsValue = field(event, 'mods');
   let modifiers = 0;
   if (modsValue !== undefined) {
      if (!Array.isArray(modsValue)) failShape('event mods');
      for (const mod of modsValue) modifiers |= MOD_BIT[requiredString(mod, 'event modifier')] ?? 0;
   }
   const rawButton = field(event, 'button');
   const rawClicks = field(event, 'clicks');
   const button = u32(rawButton === undefined ? 0 : rawButton, "event 'button' must be a u32");
   const clicks = u32(rawClicks === undefined ? 0 : rawClicks, "event 'clicks' must be a u32");
   return [
      eventType,
      Number(field(event, 'x') ?? 0),
      Number(field(event, 'y') ?? 0),
      Number(field(event, 'dx') ?? 0),
      Number(field(event, 'dy') ?? 0),
      button,
      String(field(event, 'key') ?? ''),
      String(field(event, 'text') ?? ''),
      modifiers,
      clicks,
   ];
}

function signalsFromDispatch(json: string): Signal[] {
   const parsed: unknown = JSON.parse(json);
   const effects = objectValue(parsed, 'dispatch effects');
   const signals = field(effects, 'signals');
   if (!Array.isArray(signals)) failShape('dispatch effects signals');
   return signals.map((value, index) => {
      const signal = objectValue(value, `dispatch effects signals[${index}]`);
      return {
         name: requiredString(field(signal, 'name'), `dispatch signal ${index}.name`),
         text: requiredString(field(signal, 'text'), `dispatch signal ${index}.text`),
         item: requiredString(field(signal, 'item'), `dispatch signal ${index}.item`),
      };
   });
}

function drainFrameSignals(inst: KInst, lines: string[], signals: Signal[]): void {
   const drain: unknown = inst;
   if (
      drain === null ||
      typeof drain !== 'object' ||
      !('take_signals_dump_json' in drain) ||
      typeof drain.take_signals_dump_json !== 'function'
   ) {
      failShape('kernel settled-frame signal drain');
   }
   const dump = drain.take_signals_dump_json();
   const pending = signalsFromDispatch(dump);
   if (pending.length === 0) return;
   signals.push(...pending);
   lines.push(dump);
}

function checkTraceExpectations(
   expected: TraceExpect | undefined,
   signals: Signal[],
   summaryJson: string,
): void {
   if (!expected) return;
   if (expected.signals) {
      if (expected.signals.length !== signals.length) {
         throw new Error(`expected ${expected.signals.length} signals, got ${signals.length}`);
      }
      for (let index = 0; index < expected.signals.length; index++) {
         const want = expected.signals[index];
         const got = signals[index];
         if (got.name !== want.name || got.text !== want.text || got.item !== want.item) {
            throw new Error(
               `signal ${index}: expected ${JSON.stringify(want)}, got ${JSON.stringify(got)}`,
            );
         }
      }
   }

   const summary = objectValue(JSON.parse(summaryJson), 'trace summary');
   const actualFocus = field(summary, 'focus');
   if (
      expected.focusKey !== undefined &&
      JSON.stringify(actualFocus) !== JSON.stringify(expected.focusKey)
   ) {
      throw new Error(
         `focus: expected ${JSON.stringify(expected.focusKey)}, got ${JSON.stringify(actualFocus)}`,
      );
   }

   const actualEdits = field(summary, 'edits');
   const editList = Array.isArray(actualEdits) ? actualEdits : [];
   for (const want of expected.edits ?? []) {
      const found = editList.some((item) => {
         if (item === null || typeof item !== 'object' || Array.isArray(item)) return false;
         return field(item, 'name') === want.name && field(item, 'text') === want.text;
      });
      if (!found) {
         throw new Error(
            `edit ${JSON.stringify(want.name)}: expected ${JSON.stringify(want.text)}, summary ${JSON.stringify(actualEdits)}`,
         );
      }
   }

   const actualScroll = field(summary, 'scroll');
   const scrollList = Array.isArray(actualScroll) ? actualScroll : [];
   for (const want of expected.scroll ?? []) {
      const found = scrollList.some((item) => {
         if (item === null || typeof item !== 'object' || Array.isArray(item)) return false;
         const off = field(item, 'off');
         return (
            field(item, 'key') === want.key &&
            u32Or(field(item, 'axis'), -1) === want.axis &&
            typeof off === 'number' &&
            Math.abs(off - want.off) < 1e-6
         );
      });
      if (!found) {
         throw new Error(
            `scroll ${JSON.stringify(want.key)} axis ${want.axis}: expected ${want.off}, summary ${JSON.stringify(actualScroll)}`,
         );
      }
   }
}

function runTrace(
   bytes: Uint8Array,
   trace: TraceCase,
): { output: string; signals: Signal[]; summary: string } {
   const inst = new KInst(bytes);
   try {
      const statics = parseStatics(inst.statics_json());
      const env: RuntimeEnv = {
         vw: trace.env.vw ?? 800,
         vh: trace.env.vh ?? 0,
         client: clientCode(trace.env.client ?? 'web'),
         dark: trace.env.dark ?? false,
         coarse: trace.env.coarse ?? false,
      };
      inst.set_env(env.vw, env.vh, env.client, env.dark, env.coarse);
      for (const param of trace.params) {
         const index = paramIndex(statics, param.name);
         if (index < 0) throw new Error(`unknown param '${param.name}'`);
         if (!setParam(inst, index, traceParamValue(param))) {
            throw new Error(`param '${param.name}' rejected`);
         }
      }
      const steps = trace.steps;
      let lastTime = steps.length > 0 ? (steps[0].time ?? 0) : 0;
      const lines: string[] = [];
      const signals: Signal[] = [];
      solve(inst, lastTime);
      drainFrameSignals(inst, lines, signals);
      for (const step of steps) {
         const time = step.time ?? lastTime;
         lastTime = time;
         solve(inst, time);
         drainFrameSignals(inst, lines, signals);
         if (step.event) {
            const args = eventArgs(step.event);
            const dump = inst.dispatch_dump_json(...args);
            signals.push(...signalsFromDispatch(dump));
            lines.push(dump);
         } else if (step.state) {
            if (step.state.key !== undefined) {
               if (!inst.set_node_state(step.state.key, step.state.name, step.state.on ?? true)) {
                  throw new Error(`unknown node key '${step.state.key}'`);
               }
            } else {
               inst.set_state(step.state.name, step.state.on ?? true);
            }
            lines.push('{"set":"state"}');
         } else if (step.env) {
            env.vw = step.env.vw ?? env.vw;
            env.vh = step.env.vh ?? env.vh;
            env.client = step.env.client === undefined ? env.client : clientCode(step.env.client);
            env.dark = step.env.dark ?? env.dark;
            env.coarse = step.env.coarse ?? env.coarse;
            inst.set_env(env.vw, env.vh, env.client, env.dark, env.coarse);
            lines.push('{"set":"env"}');
         } else if (step.param) {
            const index = paramIndex(statics, step.param.name);
            if (index < 0) throw new Error(`unknown param '${step.param.name}'`);
            const ok = setParam(inst, index, traceParamValue(step.param));
            lines.push(`{"set":"param","ok":${ok}}`);
         } else if (step.img) {
            if (step.img.op === 'unregister') {
               const ok = inst.img_unregister(step.img.name);
               lines.push(`{"set":"img","op":"unregister","ok":${ok}}`);
            } else {
               const img = inst.img_register(
                  step.img.name,
                  step.img.w,
                  step.img.h,
                  step.img.format,
                  step.img.bytes,
               );
               lines.push(`{"set":"img","img":${img}}`);
            }
         } else if (step.scroll) {
            const axis = step.scroll.axis === undefined ? 0 : u32Or(step.scroll.axis, 0xffff_ffff);
            const changed = inst.set_scroll(step.scroll.key, axis, step.scroll.off);
            const read = inst.get_scroll(step.scroll.key, axis) === step.scroll.off;
            lines.push(`{"set":"scroll","changed":${changed},"read":${read}}`);
         } else if (step.list) {
            const found = paramIndex(statics, step.list.param);
            const param = found < 0 ? 0xffff_ffff : found;
            const op = step.list.op;
            let ok: boolean;
            if (op === 'len') {
               const n = i32Or(step.list.n, -0x8000_0000);
               ok =
                  inst.set_list_len(param, step.list.path, n) &&
                  inst.list_len(param, step.list.path) === n;
            } else if (op === 'field') {
               ok = setListField(
                  inst,
                  param,
                  step.list.path,
                  i32Or(step.list.index, -0x8000_0000),
                  step.list.field ?? '',
                  traceParamValue({
                     name: '',
                     kind: step.list.kind ?? '',
                     value: step.list.value,
                  }),
               );
            } else if (op === 'key') {
               ok = inst.set_list_key(
                  param,
                  step.list.path,
                  i32Or(step.list.index, -0x8000_0000),
                  step.list.key ?? '',
               );
            } else {
               throw new Error(`unknown list op '${op}'`);
            }
            lines.push(`{"set":"list","op":"${op}","ok":${ok}}`);
         } else if (step.divider) {
            const changed = inst.set_divider(step.divider.key, step.divider.extent);
            const read = inst.get_divider(step.divider.key) === step.divider.extent;
            lines.push(`{"set":"divider","changed":${changed},"read":${read}}`);
         } else if (step.reveal) {
            const ok = inst.reveal(step.reveal.key, step.reveal.margin);
            lines.push(`{"set":"reveal","ok":${ok}}`);
         } else if (step.revealItem) {
            const ok = inst.reveal_item(
               step.revealItem.each,
               i32Or(step.revealItem.index, -0x8000_0000),
               u32Or(step.revealItem.align, 0xffff_ffff),
            );
            lines.push(`{"set":"reveal_item","ok":${ok}}`);
         } else if (step.window) {
            lines.push(`{"window":${inst.each_window_json(step.window.each)}}`);
         } else if (step.focus) {
            const ok = inst.set_focus(step.focus.key, step.focus.visible ?? true);
            lines.push(`{"set":"focus","ok":${ok}}`);
         } else if (step.hole) {
            inst.set_hole_size(step.hole.hole, step.hole.w, step.hole.h);
            lines.push('{"set":"hole"}');
         } else if (step.hit) {
            lines.push(inst.hit_json(step.hit[0], step.hit[1]));
         } else {
            lines.push('{"tick":true}');
         }
      }
      const frameJson = inst.frame_json(lastTime);
      drainFrameSignals(inst, lines, signals);
      const summary = inst.trace_summary_json();
      const stepOutput = lines.map((line) => `${line}\n`).join('');
      return { output: `${stepOutput}${summary}\n${frameJson}\n`, signals, summary };
   } finally {
      inst.free();
   }
}

function byteDiff(gotText: string, want: Uint8Array): string | null {
   const got = new TextEncoder().encode(gotText);
   const length = Math.min(got.length, want.length);
   let at = length;
   for (let index = 0; index < length; index++) {
      if (got[index] !== want[index]) {
         at = index;
         break;
      }
   }
   if (at === length && got.length === want.length) return null;
   const low = Math.max(0, at - 60);
   const high = at + 60;
   const decoder = new TextDecoder();
   return (
      `first diff at byte ${at} (expected ${want.length} bytes, got ${got.length})\n` +
      `  expected: …${decoder.decode(want.slice(low, high))}…\n` +
      `  got:      …${decoder.decode(got.slice(low, high))}…`
   );
}

async function compareGolden(payload: string, path: string): Promise<string | null> {
   const bytes = new Uint8Array(await Bun.file(path).arrayBuffer());
   return byteDiff(payload, bytes);
}

async function runTraces(): Promise<{ pass: number; fail: number }> {
   const dir = join(root, 'conformance/cases/traces');
   let names: string[];
   try {
      names = (await readdir(dir))
         .filter((name) => name.endsWith('.json'))
         .map((name) => name.slice(0, -5))
         .sort();
   } catch {
      return { pass: 0, fail: 0 };
   }
   let pass = 0;
   let fail = 0;
   for (const name of names) {
      try {
         const trace = parseTraceCase(await readJson(join(dir, `${name}.json`)));
         const bytes = await ensureTraceSlir(trace.doc);
         const result = runTrace(bytes, trace);
         checkTraceExpectations(trace.expect, result.signals, result.summary);
         const mismatch = await compareGolden(
            result.output,
            join(root, 'conformance/expected/traces', `${name}.trace.txt`),
         );
         if (mismatch === null) {
            console.error(`ok trace ${name}`);
            pass++;
         } else {
            console.error(`FAIL trace ${name}: trace mismatch (kernel-wasm vs golden)`);
            console.error(mismatch);
            fail++;
         }
      } catch (error) {
         console.error(`FAIL trace ${name}: ${error}`);
         fail++;
      }
   }
   return { pass, fail };
}

function countField(counts: object, name: string): number {
   return requiredNumber(field(counts, name), `selftest counts.${name}`);
}

async function selftest(): Promise<number> {
   const slirPath = join(slirDir, 'selftest-settings.slir');
   const src = join(root, 'examples/10-settings.slab');
   const out = await Bun.$`cargo run -q -p slab-cli -- build ${src} -o ${slirPath}`
      .cwd(root)
      .quiet()
      .nothrow();
   if (out.exitCode !== 0) {
      console.error(`selftest: compile failed: ${out.stderr.toString()}`);
      return 1;
   }
   const bytes = new Uint8Array(await Bun.file(slirPath).arrayBuffer());
   let inst: KInst;
   try {
      inst = new KInst(bytes);
   } catch (error) {
      console.error(`selftest: SLIR decode failed: ${error}`);
      return 1;
   }
   try {
      inst.set_env(800, 600, 0, false, false);
      const parsed: unknown = JSON.parse(inst.selftest_counts_json(0));
      const counts = objectValue(parsed, 'selftest counts');
      const nodes = countField(counts, 'nodes');
      const strings = countField(counts, 'strs');
      const values = countField(counts, 'values');
      const fonts = countField(counts, 'fonts');
      const ops = countField(counts, 'ops');
      if (nodes === 0 || strings === 0 || values === 0 || fonts === 0 || ops === 0) {
         console.error('selftest: decoded settings fixture is unexpectedly empty');
         return 1;
      }
      JSON.parse(inst.frame_json(0));
      console.error(`selftest: ok (${nodes} nodes, ${fonts} fonts, ${ops} ops)`);
      return 0;
   } catch (error) {
      console.error(`selftest: ${error}`);
      return 1;
   } finally {
      inst.free();
   }
}

async function main(): Promise<number> {
   if (process.argv.includes('--selftest')) return selftest();
   const cases = await readManifest();
   let pass = 0;
   let fail = 0;
   for (const testCase of cases) {
      try {
         const bytes = await ensureSlir(testCase.name, testCase.source);
         const outputs = runManifestCase(bytes, testCase);
         let matched = true;
         for (const output of outputs) {
            const mismatch = await compareGolden(
               output.payload,
               join(root, 'conformance/expected', output.name),
            );
            if (mismatch !== null) {
               console.error(
                  `FAIL ${testCase.name}: ${output.name} mismatch (kernel-wasm vs golden)`,
               );
               console.error(mismatch);
               matched = false;
            }
         }
         if (matched) {
            console.error(`ok ${testCase.name}`);
            pass++;
         } else {
            fail++;
         }
      } catch (error) {
         console.error(`FAIL ${testCase.name}: ${error}`);
         fail++;
      }
   }
   const traces = await runTraces();
   pass += traces.pass;
   fail += traces.fail;
   console.error(`conformance-wasm: ${pass}/${cases.length + traces.pass + traces.fail} ok`);
   return fail === 0 ? 0 : 1;
}

process.exit(await main());
