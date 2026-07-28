import { type FSWatcher, mkdirSync, watch, writeFileSync } from 'node:fs';
import { createServer, type Server, type ServerResponse } from 'node:http';
import { dirname, extname, relative, resolve, sep } from 'node:path';

export const DEV_USAGE =
   'usage: slab dev FILE [-o DIR] [--tag NAME] [--separate-ir] [--host HOST] [--port N]\n';

export type DevOptions = {
   file: string;
   out: string;
   tag?: string;
   separateIr: boolean;
   host: string;
   port: number;
};

export type GeneratedFile = { name: string; bytes: Buffer; text?: string };
export type DevBuildResult = {
   files: GeneratedFile[];
   diagnostics: string[];
   hasErrors: boolean;
};
export type DevGenerator = () => DevBuildResult;

export class DevUsageError extends Error {}

function optionValue(args: string[], index: number, flag: string): string {
   const value = args[index + 1];
   if (value === undefined || value.startsWith('-')) {
      throw new DevUsageError(`missing value for ${flag}`);
   }
   return value;
}

export function parseDevArgs(args: string[]): DevOptions {
   let file: string | undefined;
   let out: string | undefined;
   let tag: string | undefined;
   let host = '127.0.0.1';
   let port = 3000;
   let separateIr = false;

   for (let index = 0; index < args.length; index += 1) {
      const argument = args[index] as string;
      if (argument === '-o' || argument === '--out') {
         out = optionValue(args, index, argument);
         index += 1;
      } else if (argument === '--tag') {
         tag = optionValue(args, index, argument);
         index += 1;
      } else if (argument === '--host') {
         host = optionValue(args, index, argument);
         index += 1;
      } else if (argument === '--port') {
         const value = optionValue(args, index, argument);
         if (!/^\d+$/.test(value))
            throw new DevUsageError('--port must be an integer from 0 to 65535');
         port = Number(value);
         if (port > 65_535) throw new DevUsageError('--port must be an integer from 0 to 65535');
         index += 1;
      } else if (argument === '--separate-ir') {
         separateIr = true;
      } else if (argument.startsWith('-')) {
         throw new DevUsageError(`unknown flag ${argument}`);
      } else if (file === undefined) {
         file = argument;
      } else {
         throw new DevUsageError(`unexpected argument '${argument}'`);
      }
   }

   if (file === undefined) throw new DevUsageError('dev needs FILE');
   if (host.length === 0) throw new DevUsageError('--host cannot be empty');
   const source = resolve(file);
   const extension = extname(source);
   const defaultOut = `${source.slice(0, source.length - extension.length)}.dev`;
   const output = resolve(out ?? defaultOut);
   if (isWithin(dirname(source), output)) {
      throw new DevUsageError('output directory cannot contain the source directory');
   }
   return {
      file: source,
      out: output,
      tag,
      separateIr,
      host,
      port,
   };
}

function isWithin(path: string, directory: string): boolean {
   const rel = relative(directory, path);
   return rel === '' || (rel !== '..' && !rel.startsWith(`..${sep}`));
}

function isTemporary(path: string): boolean {
   const name = path.split(/[\\/]/).pop() ?? '';
   return (
      name === '' ||
      name.startsWith('.#') ||
      (name.startsWith('#') && name.endsWith('#')) ||
      name.endsWith('~') ||
      /\.(?:swp|swo|tmp|temp)$/.test(name)
   );
}

function contentType(path: string): string {
   switch (extname(path)) {
      case '.js':
         return 'text/javascript; charset=utf-8';
      case '.json':
      case '.map':
         return 'application/json; charset=utf-8';
      case '.wasm':
         return 'application/wasm';
      case '.css':
         return 'text/css; charset=utf-8';
      case '.svg':
         return 'image/svg+xml';
      case '.png':
         return 'image/png';
      default:
         return 'application/octet-stream';
   }
}

function previewHtml(stem: string, tag: string, hasBuild: boolean, failure: string): string {
   const safeTag = JSON.stringify(tag).slice(1, -1).replaceAll('<', '\\u003c');
   const module = hasBuild
      ? `<script type="module" src="/assets/${encodeURIComponent(stem)}.js"></script>`
      : '';
   const element = hasBuild ? `<${safeTag}></${safeTag}>` : '';
   const initialFailure = JSON.stringify(failure).replaceAll('<', '\\u003c');
   return `<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Slab preview</title><style>
html,body{margin:0;min-height:100%;background:#fff}body>:not(script):not(slab-dev-error){display:block;width:100vw;height:100vh}slab-dev-error{position:fixed;z-index:2147483647;left:12px;right:12px;bottom:12px;max-height:45vh;overflow:auto;white-space:pre-wrap;padding:12px 14px;border:1px solid #b42318;border-radius:6px;background:#fff4f2;color:#7a271a;font:12px/1.45 ui-monospace,monospace;box-shadow:0 4px 18px #0003}slab-dev-error:empty{display:none}
</style>${module}</head><body>${element}<slab-dev-error></slab-dev-error><script>
const error=document.querySelector('slab-dev-error');error.textContent=${initialFailure};
const events=new EventSource('/__slab/events');
events.addEventListener('reload',()=>location.reload());
events.addEventListener('build-error',event=>{error.textContent=JSON.parse(event.data)});
</script></body></html>`;
}

export type DevSession = {
   url: string;
   close(): Promise<void>;
};

export async function startDevServer(
   options: DevOptions,
   generate: DevGenerator,
   log: (line: string) => void = (line) => process.stderr.write(`${line}\n`),
): Promise<DevSession> {
   const sourceDirectory = resolve(options.file, '..');
   const outputDirectory = resolve(options.out);
   const stem = options.file.slice(options.file.lastIndexOf(sep) + 1).replace(/\.[^.]+$/, '');
   let previewTag = options.tag ?? `slab-${stem.toLowerCase().replace(/[^a-z0-9]+/g, '-')}`;
   let latestFailure = '';
   let hasBuild = false;
   let closed = false;
   let watchReady = false;
   let pending: NodeJS.Timeout | undefined;
   let building = false;
   let dirty = false;
   const files = new Map<string, Buffer>();
   const clients = new Set<ServerResponse>();

   const broadcast = (event: string, data: string): void => {
      const frame = `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`;
      for (const client of clients) {
         if (client.destroyed) clients.delete(client);
         else client.write(frame);
      }
   };

   const build = (): boolean => {
      let result: DevBuildResult;
      try {
         result = generate();
      } catch (error) {
         latestFailure = String(error);
         log(latestFailure);
         broadcast('build-error', latestFailure);
         return false;
      }
      if (result.diagnostics.length > 0) log(result.diagnostics.join('\n'));
      if (result.hasErrors) {
         latestFailure = result.diagnostics.join('\n') || 'Compilation failed';
         broadcast('build-error', latestFailure);
         return false;
      }
      const next = new Map<string, Buffer>();
      for (const file of result.files) {
         const normalized = file.name.replaceAll('\\', '/');
         if (normalized.startsWith('/') || normalized.split('/').includes('..')) continue;
         next.set(normalized, file.bytes);
         const outputPath = resolve(outputDirectory, normalized);
         if (!isWithin(outputPath, outputDirectory)) continue;
         mkdirSync(resolve(outputPath, '..'), { recursive: true });
         writeFileSync(outputPath, file.bytes);
         if (file.name === `${stem}.js`) {
            const match = file.text?.match(/customElements\.define\(['"]([^'"]+)['"]/);
            if (match?.[1]) previewTag = match[1];
         }
      }
      files.clear();
      for (const [name, bytes] of next) files.set(name, bytes);
      latestFailure = '';
      hasBuild = true;
      return true;
   };

   build();

   const server: Server = createServer((request, response) => {
      const url = new URL(request.url ?? '/', 'http://localhost');
      if (url.pathname === '/') {
         response.writeHead(200, {
            'content-type': 'text/html; charset=utf-8',
            'cache-control': 'no-store',
         });
         response.end(previewHtml(stem, previewTag, hasBuild, latestFailure));
         return;
      }
      if (url.pathname === '/__slab/events') {
         response.writeHead(200, {
            'content-type': 'text/event-stream',
            'cache-control': 'no-store',
            connection: 'keep-alive',
         });
         response.write(': connected\n\n');
         clients.add(response);
         request.on('close', () => clients.delete(response));
         return;
      }
      if (url.pathname === '/__slab/status') {
         response.writeHead(200, {
            'content-type': 'application/json; charset=utf-8',
            'cache-control': 'no-store',
         });
         response.end(JSON.stringify({ ok: latestFailure === '', diagnostics: latestFailure }));
         return;
      }
      if (url.pathname.startsWith('/assets/')) {
         let name: string;
         try {
            name = decodeURIComponent(url.pathname.slice('/assets/'.length));
         } catch {
            response.writeHead(400).end();
            return;
         }
         const bytes = name.includes('..') ? undefined : files.get(name);
         if (bytes !== undefined) {
            response.writeHead(200, {
               'content-type': contentType(name),
               'cache-control': 'no-store',
            });
            response.end(bytes);
            return;
         }
      }
      response.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' });
      response.end('Not found\n');
   });

   await new Promise<void>((resolveReady, reject) => {
      server.once('error', reject);
      server.listen(options.port, options.host, () => {
         server.off('error', reject);
         resolveReady();
      });
   });

   const address = server.address();
   if (address === null || typeof address === 'string')
      throw new Error('development server has no TCP address');
   const displayHost = options.host.includes(':') ? `[${options.host}]` : options.host;
   const url = `http://${displayHost}:${address.port}/`;

   const rebuild = (): void => {
      if (building) {
         dirty = true;
         return;
      }
      building = true;
      do {
         dirty = false;
         if (build()) broadcast('reload', '');
      } while (dirty);
      building = false;
   };

   const schedule = (filename: string | null): void => {
      if (closed || !watchReady) return;
      if (filename !== null) {
         const changed = resolve(sourceDirectory, filename);
         if (isWithin(changed, outputDirectory) || isTemporary(changed)) return;
      }
      clearTimeout(pending);
      pending = setTimeout(rebuild, 75);
   };

   let watcher: FSWatcher;
   try {
      watcher = watch(sourceDirectory, { recursive: true }, (_event, filename) =>
         schedule(filename === null ? null : String(filename)),
      );
   } catch (error) {
      await new Promise<void>((resolveClose) => server.close(() => resolveClose()));
      throw error;
   }

   // macOS can replay the output directory creation as the watcher's first
   // recursive event. Drain that event before callers can make their first edit.
   await new Promise<void>((resolveWatch) => setTimeout(resolveWatch, 100));
   watchReady = true;

   const close = async (): Promise<void> => {
      if (closed) return;
      closed = true;
      clearTimeout(pending);
      watcher.close();
      for (const client of clients) client.end();
      clients.clear();
      await new Promise<void>((resolveClose, reject) => {
         server.close((error) => (error ? reject(error) : resolveClose()));
      });
   };

   return { url, close };
}
