import type { Frame, FrameDiagnostic, FrameOp, RtPath } from './kernel.ts';
import type { FrameBuf } from './wasm/slab_kernel.js';

function signedWord(word: number): number {
   return word > 0x7fffffff ? word - 0x100000000 : word;
}

function decodeStrings(json: string): string[] {
   const value: unknown = JSON.parse(json);
   if (!Array.isArray(value)) throw new Error('invalid FrameBuf string pool: expected an array');
   const strings: string[] = [];
   for (const item of value) {
      if (typeof item !== 'string')
         throw new Error('invalid FrameBuf string pool: expected only strings');
      strings.push(item);
   }
   return strings;
}

function decodeRuntimePaths(json: string): RtPath[] {
   const value: unknown = JSON.parse(json);
   if (!Array.isArray(value)) throw new Error('invalid FrameBuf runtime paths: expected an array');
   const paths: RtPath[] = [];
   for (let index = 0; index < value.length; index++) {
      const pair = value[index];
      if (!Array.isArray(pair) || pair.length !== 2) {
         throw new Error(`invalid FrameBuf runtime path ${index}: expected [verbs, coords]`);
      }
      const [verbs, coords] = pair;
      if (
         !Array.isArray(verbs) ||
         verbs.some(
            (verb) =>
               typeof verb !== 'number' || !Number.isInteger(verb) || verb < 0 || verb > 0xff,
         )
      ) {
         throw new Error(`invalid FrameBuf runtime path ${index}: invalid verbs`);
      }
      if (
         !Array.isArray(coords) ||
         coords.some((coord) => typeof coord !== 'number' || !Number.isFinite(coord))
      ) {
         throw new Error(`invalid FrameBuf runtime path ${index}: invalid coordinates`);
      }
      paths.push({ verbs: verbs as number[], coords: coords as number[] });
   }
   return paths;
}

function decodeDiagnostics(json: string): FrameDiagnostic[] {
   const value: unknown = JSON.parse(json);
   if (!Array.isArray(value)) throw new Error('invalid FrameBuf diagnostics: expected an array');
   const diagnostics: FrameDiagnostic[] = [];
   for (let index = 0; index < value.length; index++) {
      const item: unknown = value[index];
      if (typeof item !== 'object' || item === null || Array.isArray(item)) {
         throw new Error(`invalid FrameBuf diagnostic ${index}: expected {code,line,msg}`);
      }
      const record = item as Record<string, unknown>;
      if (
         typeof record.code !== 'string' ||
         typeof record.line !== 'number' ||
         !Number.isInteger(record.line) ||
         record.line < 0 ||
         typeof record.msg !== 'string'
      ) {
         throw new Error(`invalid FrameBuf diagnostic ${index}: expected {code,line,msg}`);
      }
      diagnostics.push({ code: record.code, line: record.line, msg: record.msg });
   }
   return diagnostics;
}

/** Consumes a bindgen frame buffer and reconstructs the painter operation stream. */
export function decodeFrame(frame: FrameBuf): Frame {
   try {
      const words = frame.u32s();
      const floats = frame.f64s();
      const strings = decodeStrings(frame.strs_json());
      const pathsRt = decodeRuntimePaths(frame.rt_paths_json());
      const dirty = frame.dirty();
      const motionActive = frame.motion_active();
      const diagnostics = decodeDiagnostics(frame.diagnostics_json());
      const uncovered = frame.uncovered_u32s();
      let wi = 0;
      let fi = 0;

      const u32 = (description: string): number => {
         if (wi >= words.length)
            throw new Error(`truncated FrameBuf u32 payload while reading ${description}`);
         return words[wi++];
      };
      const f64 = (description: string): number => {
         if (fi >= floats.length)
            throw new Error(`truncated FrameBuf f64 payload while reading ${description}`);
         return floats[fi++];
      };

      const width = f64('frame width');
      const height = f64('frame height');
      const ops: FrameOp[] = [];
      while (wi < words.length) {
         const tag = u32('operation tag');
         switch (tag) {
            case 0:
               ops.push({
                  tag: 'Rect',
                  v: {
                     node: u32('Rect.node'),
                     bg_kind: u32('Rect.bg_kind'),
                     bg: u32('Rect.bg'),
                     stroke_kind: u32('Rect.stroke_kind'),
                     stroke: u32('Rect.stroke'),
                     stroke_align: u32('Rect.stroke_align'),
                     stroke_sides: u32('Rect.stroke_sides'),
                     has_dash: u32('Rect.has_dash') !== 0,
                     shadow_off: signedWord(u32('Rect.shadow_off')),
                     shadow_len: signedWord(u32('Rect.shadow_len')),
                     x: f64('Rect.x'),
                     y: f64('Rect.y'),
                     w: f64('Rect.w'),
                     h: f64('Rect.h'),
                     radius: f64('Rect.radius'),
                     stroke_w: f64('Rect.stroke_w'),
                     dash_on: f64('Rect.dash_on'),
                     dash_off: f64('Rect.dash_off'),
                     opacity: f64('Rect.opacity'),
                     smooth: f64('Rect.smooth'),
                     grain_amount: f64('Rect.grain_amount'),
                     grain_size: f64('Rect.grain_size'),
                  },
               });
               break;
            case 1:
               ops.push({
                  tag: 'Text',
                  v: {
                     node: u32('Text.node'),
                     str_ref: signedWord(u32('Text.str_ref')),
                     font: signedWord(u32('Text.font')),
                     weight: u32('Text.weight'),
                     color: u32('Text.color'),
                     color_kind: u32('Text.color_kind'),
                     strike: u32('Text.strike') !== 0,
                     uncov_off: signedWord(u32('Text.uncov_off')),
                     uncov_len: u32('Text.uncov_len'),
                     x: f64('Text.x'),
                     y_baseline: f64('Text.y_baseline'),
                     measured_w: f64('Text.measured_w'),
                     size: f64('Text.size'),
                     tracking: f64('Text.tracking'),
                     opacity: f64('Text.opacity'),
                     gx: f64('Text.gx'),
                     gy: f64('Text.gy'),
                     gw: f64('Text.gw'),
                     gh: f64('Text.gh'),
                  },
               });
               break;
            case 2:
               ops.push({
                  tag: 'Image',
                  v: {
                     node: u32('Image.node'),
                     img: signedWord(u32('Image.img')),
                     fit: u32('Image.fit'),
                     x: f64('Image.x'),
                     y: f64('Image.y'),
                     w: f64('Image.w'),
                     h: f64('Image.h'),
                     radius: f64('Image.radius'),
                     opacity: f64('Image.opacity'),
                     smooth: f64('Image.smooth'),
                  },
               });
               break;
            case 3:
               ops.push({
                  tag: 'PathDraw',
                  v: {
                     node: u32('PathDraw.node'),
                     path: signedWord(u32('PathDraw.path')),
                     bg_kind: u32('PathDraw.bg_kind'),
                     bg: u32('PathDraw.bg'),
                     stroke_kind: u32('PathDraw.stroke_kind'),
                     stroke: u32('PathDraw.stroke'),
                     has_dash: u32('PathDraw.has_dash') !== 0,
                     dx: f64('PathDraw.dx'),
                     dy: f64('PathDraw.dy'),
                     stroke_w: f64('PathDraw.stroke_w'),
                     dash_on: f64('PathDraw.dash_on'),
                     dash_off: f64('PathDraw.dash_off'),
                     opacity: f64('PathDraw.opacity'),
                  },
               });
               break;
            case 4:
               ops.push({
                  tag: 'ClipPush',
                  v: {
                     x: f64('ClipPush.x'),
                     y: f64('ClipPush.y'),
                     w: f64('ClipPush.w'),
                     h: f64('ClipPush.h'),
                     radius: f64('ClipPush.radius'),
                     smooth: f64('ClipPush.smooth'),
                  },
               });
               break;
            case 5:
               ops.push({ tag: 'ClipPop' });
               break;
            case 6:
               ops.push({
                  tag: 'GroupPush',
                  v: {
                     node: u32('GroupPush.node'),
                     mask_kind: u32('GroupPush.mask_kind'),
                     mask: u32('GroupPush.mask'),
                     opacity: f64('GroupPush.opacity'),
                     blur: f64('GroupPush.blur'),
                     mx: f64('GroupPush.mx'),
                     my: f64('GroupPush.my'),
                     mw: f64('GroupPush.mw'),
                     mh: f64('GroupPush.mh'),
                  },
               });
               break;
            case 7:
               ops.push({ tag: 'GroupPop' });
               break;
            case 8:
               ops.push({
                  tag: 'RotatePush',
                  v: {
                     cx: f64('RotatePush.cx'),
                     cy: f64('RotatePush.cy'),
                     deg: f64('RotatePush.deg'),
                  },
               });
               break;
            case 9:
               ops.push({ tag: 'RotatePop' });
               break;
            case 10:
               ops.push({
                  tag: 'Backdrop',
                  v: {
                     mask_kind: u32('Backdrop.mask_kind'),
                     mask: u32('Backdrop.mask'),
                     x: f64('Backdrop.x'),
                     y: f64('Backdrop.y'),
                     w: f64('Backdrop.w'),
                     h: f64('Backdrop.h'),
                     radius: f64('Backdrop.radius'),
                     blur: f64('Backdrop.blur'),
                     saturate: f64('Backdrop.saturate'),
                     brightness: f64('Backdrop.brightness'),
                     smooth: f64('Backdrop.smooth'),
                  },
               });
               break;
            case 11:
               ops.push({
                  tag: 'ScalePush',
                  v: {
                     cx: f64('ScalePush.cx'),
                     cy: f64('ScalePush.cy'),
                     sx: f64('ScalePush.sx'),
                     sy: f64('ScalePush.sy'),
                  },
               });
               break;
            case 12:
               ops.push({ tag: 'ScalePop' });
               break;
            case 13:
               ops.push({
                  tag: 'TiltPush',
                  v: {
                     cx: f64('TiltPush.cx'),
                     cy: f64('TiltPush.cy'),
                     rx: f64('TiltPush.rx'),
                     ry: f64('TiltPush.ry'),
                     depth: f64('TiltPush.depth'),
                  },
               });
               break;
            case 14:
               ops.push({ tag: 'TiltPop' });
               break;
            default:
               throw new Error(`unknown FrameBuf operation tag ${tag} at u32 word ${wi - 1}`);
         }
      }
      if (fi !== floats.length)
         throw new Error(`invalid FrameBuf: ${floats.length - fi} trailing f64 values`);

      return {
         width,
         height,
         ops,
         strings,
         pathsRt,
         dirty,
         motionActive,
         diagnostics,
         uncovered,
      };
   } finally {
      frame.free();
   }
}
