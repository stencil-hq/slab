// Shared static-file plumbing for the local dev servers (serve.ts, dev.ts).

import { extname, join } from 'node:path';
import Bun from 'bun';

const MIME: Record<string, string> = {
   '.html': 'text/html',
   '.js': 'text/javascript',
   '.css': 'text/css',
   '.json': 'application/json',
   '.wasm': 'application/wasm',
   '.png': 'image/png',
   '.ttf': 'font/ttf',
   '.slab': 'text/plain',
};

/** A resolved static asset: the file body plus its content type. */
export interface StaticHit {
   file: Blob;
   mime: string;
}

/** Resolve `pathname` under `root` (extensionless paths fall back to index.html). */
export async function resolveStatic(root: string, pathname: string): Promise<StaticHit | null> {
   let path = join(root, pathname);
   if (pathname === '/' || !path.includes('.')) path = join(root, 'index.html');
   const file = Bun.file(path);
   if (!(await file.exists())) return null;
   return { file, mime: MIME[extname(path)] ?? 'application/octet-stream' };
}
