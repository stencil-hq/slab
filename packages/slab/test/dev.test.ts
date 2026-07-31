import { afterEach, describe, expect, test } from 'bun:test';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
   type DevBuildResult,
   type DevSession,
   DevUsageError,
   parseDevArgs,
   startDevServer,
} from '../src/dev.ts';

const roots: string[] = [];
const sessions: DevSession[] = [];

afterEach(async () => {
   await Promise.all(sessions.splice(0).map((session) => session.close()));
   for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function fixture(): { root: string; source: string; out: string } {
   const root = mkdtempSync(join(tmpdir(), 'slab-dev-test-'));
   roots.push(root);
   const source = join(root, 'app.slab');
   const out = join(root, 'generated');
   writeFileSync(source, 'initial');
   return { root, source, out };
}

function result(version: string, hasErrors = false): DevBuildResult {
   const diagnostic = hasErrors ? [`error: broken ${version}`] : [];
   return {
      files: hasErrors
         ? []
         : [
              {
                 name: 'app.js',
                 text: `export const version='${version}';customElements.define('test-app', class extends HTMLElement {})`,
                 bytes: Buffer.from(
                    `export const version='${version}';customElements.define('test-app', class extends HTMLElement {})`,
                 ),
              },
           ],
      diagnostics: diagnostic,
      hasErrors,
   };
}

async function start(
   source: string,
   out: string,
   generate: () => DevBuildResult,
): Promise<DevSession> {
   const session = await startDevServer(
      { file: source, out, host: '127.0.0.1', port: 0 },
      generate,
      () => {},
   );
   sessions.push(session);
   return session;
}

class Events {
   private readonly reader: ReadableStreamDefaultReader<Uint8Array>;
   private readonly decoder = new TextDecoder();
   private buffered = '';

   private constructor(reader: ReadableStreamDefaultReader<Uint8Array>) {
      this.reader = reader;
   }

   static async connect(url: string): Promise<Events> {
      const response = await fetch(`${url}__slab/events`);
      if (response.body === null) throw new Error('event stream has no body');
      return new Events(response.body.getReader());
   }

   async next(name: string): Promise<string> {
      while (true) {
         const boundary = this.buffered.indexOf('\n\n');
         if (boundary >= 0) {
            const frame = this.buffered.slice(0, boundary);
            this.buffered = this.buffered.slice(boundary + 2);
            const event = frame.match(/^event: (.+)$/m)?.[1];
            const data = frame.match(/^data: (.+)$/m)?.[1];
            if (event === name && data !== undefined) return JSON.parse(data) as string;
            continue;
         }
         const chunk = await this.reader.read();
         if (chunk.done) throw new Error(`event stream ended before ${name}`);
         this.buffered += this.decoder.decode(chunk.value, { stream: true });
      }
   }
}

describe('parseDevArgs', () => {
   test('accepts port zero and rejects invalid arguments', () => {
      const options = parseDevArgs(['doc.slab', '--port', '0', '--host', 'localhost']);
      expect(options.port).toBe(0);
      expect(options.host).toBe('localhost');
      expect(() => parseDevArgs([])).toThrow(DevUsageError);
      expect(() => parseDevArgs(['doc.slab', '--port', '-1'])).toThrow('--port');
      expect(() => parseDevArgs(['doc.slab', '--wat'])).toThrow('unknown flag');
   });
});

describe('development server', () => {
   test('builds once and serves the preview and stable generated asset URL', async () => {
      const { source, out } = fixture();
      let builds = 0;
      const session = await start(source, out, () => {
         builds += 1;
         return result('one');
      });

      expect(builds).toBe(1);
      expect(await (await fetch(session.url)).text()).toContain('<test-app></test-app>');
      expect(await (await fetch(`${session.url}assets/app.js`)).text()).toContain("version='one'");
      expect(readFileSync(join(out, 'app.js'), 'utf8')).toContain("version='one'");
   });

   test('keeps the last good output through failure, then reloads after recovery', async () => {
      const { source, out } = fixture();
      let version = 'one';
      let broken = false;
      const session = await start(source, out, () => result(version, broken));
      const events = await Events.connect(session.url);

      broken = true;
      writeFileSync(source, 'broken');
      expect(await events.next('build-error')).toContain('broken one');
      expect(await (await fetch(`${session.url}assets/app.js`)).text()).toContain("version='one'");
      expect(await (await fetch(`${session.url}__slab/status`)).json()).toEqual({
         ok: false,
         diagnostics: 'error: broken one',
      });

      broken = false;
      version = 'two';
      writeFileSync(source, 'fixed');
      await events.next('reload');
      expect(await (await fetch(`${session.url}assets/app.js`)).text()).toContain("version='two'");
   });

   test('ignores output directory and temporary file changes', async () => {
      const { source, out, root } = fixture();
      let builds = 0;
      await start(source, out, () => {
         builds += 1;
         return result(String(builds));
      });

      mkdirSync(out, { recursive: true });
      writeFileSync(join(out, 'external.txt'), 'output change');
      writeFileSync(join(root, '.#editor-temp'), 'temporary change');
      await Bun.sleep(250);
      expect(builds).toBe(1);
   });
});
