export interface FontMetrics {
   weight: number;
   upem: number;
   ascent: number;
   descent: number;
   lineGap: number;
   defaultAdvance: number;
   underlinePosition: number;
   underlineThickness: number;
   cps: Uint32Array;
   gids: Uint32Array;
   advs: Uint32Array;
}

interface Table {
   offset: number;
   length: number;
}

function tag(view: DataView, offset: number): string {
   return String.fromCharCode(
      view.getUint8(offset),
      view.getUint8(offset + 1),
      view.getUint8(offset + 2),
      view.getUint8(offset + 3),
   );
}

/** Parse the metrics required by `inst_font_register` from a TrueType or CFF sfnt. */
export function parseFontMetrics(bytes: Uint8Array): FontMetrics | null {
   try {
      const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
      if (view.byteLength < 12) return null;
      const signature = view.getUint32(0);
      if (signature !== 0x00010000 && signature !== 0x4f54544f) return null;
      const numTables = view.getUint16(4);
      if (12 + numTables * 16 > view.byteLength) return null;
      const tables = new Map<string, Table>();
      for (let i = 0; i < numTables; i++) {
         const offset = 12 + i * 16;
         const tableOffset = view.getUint32(offset + 8);
         const length = view.getUint32(offset + 12);
         if (tableOffset > view.byteLength || length > view.byteLength - tableOffset) return null;
         tables.set(tag(view, offset), { offset: tableOffset, length });
      }
      const head = tables.get('head');
      const hhea = tables.get('hhea');
      const maxp = tables.get('maxp');
      const cmap = tables.get('cmap');
      const hmtx = tables.get('hmtx');
      if (
         !head ||
         !hhea ||
         !maxp ||
         !cmap ||
         !hmtx ||
         head.length < 20 ||
         hhea.length < 36 ||
         maxp.length < 6
      )
         return null;

      const upem = view.getUint16(head.offset + 18);
      const ascent = view.getInt16(hhea.offset + 4);
      const descent = view.getInt16(hhea.offset + 6);
      const lineGap = view.getInt16(hhea.offset + 8);
      const hMetrics = view.getUint16(hhea.offset + 34);
      const glyphCount = view.getUint16(maxp.offset + 4);
      if (upem === 0 || hMetrics === 0 || hMetrics > glyphCount || hmtx.length < hMetrics * 4)
         return null;

      const advance = (gid: number): number | null => {
         if (gid >= glyphCount) return null;
         const metric = Math.min(gid, hMetrics - 1);
         return view.getUint16(hmtx.offset + metric * 4);
      };
      const cmapEnd = cmap.offset + cmap.length;
      if (cmap.length < 4) return null;
      const encodingCount = view.getUint16(cmap.offset + 2);
      if (cmap.offset + 4 + encodingCount * 8 > cmapEnd) return null;
      let selected: { offset: number; format: number; priority: number } | null = null;
      for (let i = 0; i < encodingCount; i++) {
         const entry = cmap.offset + 4 + i * 8;
         const platform = view.getUint16(entry);
         const encoding = view.getUint16(entry + 2);
         const offset = cmap.offset + view.getUint32(entry + 4);
         if (offset + 2 > cmapEnd) continue;
         const format = view.getUint16(offset);
         if (format !== 4 && format !== 12) continue;
         const priority =
            platform === 3 && encoding === 10 ? 2 : platform === 3 && encoding === 1 ? 1 : 0;
         if (!selected || priority > selected.priority) selected = { offset, format, priority };
      }
      if (!selected) return null;

      const mappings: [number, number][] = [];
      if (selected.format === 12) {
         if (selected.offset + 16 > cmapEnd) return null;
         const length = view.getUint32(selected.offset + 4);
         const groups = view.getUint32(selected.offset + 12);
         if (length < 16 || length > cmapEnd - selected.offset || groups > (length - 16) / 12)
            return null;
         for (let i = 0; i < groups; i++) {
            const group = selected.offset + 16 + i * 12;
            const first = view.getUint32(group);
            const last = view.getUint32(group + 4);
            const firstGid = view.getUint32(group + 8);
            if (last < first || firstGid > 0xffffffff - (last - first)) return null;
            for (let cp = first, gid = firstGid; cp <= last; cp++, gid++) {
               if (gid < glyphCount) mappings.push([cp, gid]);
            }
         }
      } else {
         if (selected.offset + 16 > cmapEnd) return null;
         const length = view.getUint16(selected.offset + 2);
         const segCount = view.getUint16(selected.offset + 6) / 2;
         if (
            length < 16 ||
            length > cmapEnd - selected.offset ||
            segCount === 0 ||
            selected.offset + 16 + segCount * 8 > selected.offset + length
         )
            return null;
         const endCodes = selected.offset + 14;
         const startCodes = endCodes + segCount * 2 + 2;
         const deltas = startCodes + segCount * 2;
         const ranges = deltas + segCount * 2;
         for (let segment = 0; segment < segCount; segment++) {
            const start = view.getUint16(startCodes + segment * 2);
            const end = view.getUint16(endCodes + segment * 2);
            const delta = view.getInt16(deltas + segment * 2);
            const range = view.getUint16(ranges + segment * 2);
            if (end < start) return null;
            for (let cp = start; cp <= end; cp++) {
               let gid: number;
               if (range === 0) {
                  gid = (cp + delta) & 0xffff;
               } else {
                  const glyphOffset = ranges + segment * 2 + range + (cp - start) * 2;
                  if (glyphOffset + 2 > selected.offset + length) return null;
                  gid = view.getUint16(glyphOffset);
                  if (gid !== 0) gid = (gid + delta) & 0xffff;
               }
               if (gid !== 0 && gid < glyphCount) mappings.push([cp, gid]);
            }
         }
      }

      const notdef = advance(0) ?? 0;
      const space = mappings.find(([cp]) => cp === 0x20);
      const spaceAdvance = space ? (advance(space[1]) ?? upem / 2) : upem / 2;
      const cps = new Uint32Array(mappings.length);
      const gids = new Uint32Array(mappings.length);
      const advs = new Uint32Array(mappings.length);
      for (let i = 0; i < mappings.length; i++) {
         const [cp, gid] = mappings[i];
         cps[i] = cp;
         gids[i] = gid;
         advs[i] = advance(gid) ?? (notdef !== 0 ? notdef : spaceAdvance);
      }
      const os2 = tables.get('OS/2');
      const post = tables.get('post');
      const underlinePosition =
         post && post.length >= 12 ? view.getInt16(post.offset + 8) : -Math.round(upem / 10);
      const underlineThickness =
         post && post.length >= 12
            ? view.getInt16(post.offset + 10)
            : Math.max(1, Math.round(upem / 20));
      return {
         weight: os2 && os2.length >= 6 ? view.getUint16(os2.offset + 4) : 400,
         upem,
         ascent,
         descent,
         lineGap,
         defaultAdvance: notdef !== 0 ? notdef : spaceAdvance,
         underlinePosition,
         underlineThickness,
         cps,
         gids,
         advs,
      };
   } catch {
      return null;
   }
}
