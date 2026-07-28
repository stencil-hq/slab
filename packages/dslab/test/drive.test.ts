import { deepEqual, equal, match, ok, rejects } from 'node:assert/strict';
import { createServer, type Server, type Socket } from 'node:net';
import { PassThrough } from 'node:stream';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { discoverSlab, run } from '../src/cli.js';
import { DriveClient, DriveRemoteError } from '../src/index.ts';

const DRIVE_FIXTURE = fileURLToPath(new URL('./drive-fixture.mjs', import.meta.url));

function object(value: unknown): value is Record<string, unknown> {
   return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function startServer(
   handle: (id: number, method: string, params: unknown, socket: Socket) => void,
): Promise<Server> {
   const server = createServer((socket) => {
      socket.setEncoding('utf8');
      let buffered = '';
      socket.on('data', (chunk: string) => {
         buffered += chunk;
         let newline = buffered.indexOf('\n');
         while (newline >= 0) {
            const line = buffered.slice(0, newline);
            buffered = buffered.slice(newline + 1);
            if (line.length > 0) {
               const raw: unknown = JSON.parse(line);
               if (!object(raw) || typeof raw.id !== 'number' || typeof raw.method !== 'string') {
                  throw new Error('invalid SDP request');
               }
               handle(raw.id, raw.method, raw.params, socket);
            }
            newline = buffered.indexOf('\n');
         }
      });
   });
   const ready = Promise.withResolvers<Server>();
   server.once('error', ready.reject);
   server.listen(0, '127.0.0.1', () => ready.resolve(server));
   return ready.promise;
}

function stopServer(server: Server): Promise<void> {
   const stopped = Promise.withResolvers<void>();
   server.close((error) => {
      if (error) stopped.reject(error);
      else stopped.resolve();
   });
   return stopped.promise;
}

function capture(): { output: PassThrough; text: () => string } {
   const output = new PassThrough();
   output.setEncoding('utf8');
   let text = '';
   output.on('data', (chunk: string) => {
      text += chunk;
   });
   return { output, text: () => text };
}

test('matches chunked TCP responses and exposes remote errors', async () => {
   const server = await startServer((id, method, _params, socket) => {
      if (method === 'protocol.info') {
         const response = JSON.stringify({
            id,
            result: { name: 'sdp', version: 1, doc: null, methods: ['protocol.info'] },
         });
         socket.write(response.slice(0, 13));
         socket.write(`${response.slice(13)}\n`);
         return;
      }
      if (method === 'state.set') {
         socket.write(
            `${JSON.stringify({ id, error: { code: -32000, message: 'state is locked' } })}\n`,
         );
         return;
      }
      if (method === 'protocol.quit') {
         socket.end(`${JSON.stringify({ id, result: { ok: true } })}\n`);
         return;
      }
      if (method === 'field.set') {
         socket.write(`${JSON.stringify({ id, result: { ok: true, changed: true } })}\n`);
         return;
      }
      if (method === 'field.get') {
         socket.write(`${JSON.stringify({ id, result: { text: 'hello' } })}\n`);
         return;
      }
      if (method === 'param.get') {
         socket.write(`${JSON.stringify({ id, result: { value: ['one', 'two'] } })}\n`);
         return;
      }
      if (method === 'focus.get') {
         socket.write(
            `${JSON.stringify({ id, result: { focus: 4, key: 'search', visible: true } })}\n`,
         );
         return;
      }
      socket.write(`${JSON.stringify({ id, result: { t: 0 } })}\n`);
   });
   const address = server.address();
   if (address === null || typeof address === 'string')
      throw new Error('test server has no TCP address');
   const client = await DriveClient.connect({ port: address.port });

   try {
      const info = await client.call('protocol.info');
      equal(info.name, 'sdp');
      deepEqual(info.methods, ['protocol.info']);

      try {
         await client.call('state.set', { name: 'disabled', on: true });
         throw new Error('state.set should reject');
      } catch (error) {
         ok(error instanceof DriveRemoteError);
         if (error instanceof DriveRemoteError) {
            equal(error.method, 'state.set');
            equal(error.code, -32000);
            match(error.message, /state is locked/);
         }
      }

      deepEqual(await client.setFieldText('search', 'hello'), { ok: true, changed: true });
      equal(await client.fieldText('search'), 'hello');
      deepEqual(await client.param('items'), ['one', 'two']);
      deepEqual(await client.focus(), { focus: 4, key: 'search', visible: true });

      deepEqual(await client.quit(), { ok: true });
   } finally {
      await stopServer(server);
   }
});

test('onLine observes every wire line in both directions', async () => {
   const fromServer = new PassThrough();
   const toServer = new PassThrough();
   const lines: Array<[string, string]> = [];
   const client = DriveClient.fromStreams(fromServer, toServer, {
      onLine: (direction, line) => lines.push([direction, line]),
   });
   const pending = client.call('clock.get');
   fromServer.write('{"id":1,"result":{"t":0}}\n');
   const clock = await pending;
   equal(clock.t, 0);
   deepEqual(lines, [
      ['send', '{"id":1,"method":"clock.get","params":{}}'],
      ['recv', '{"id":1,"result":{"t":0}}'],
   ]);
   await client.close();
});

test('drives a spawned stdio session through protocol.quit', async () => {
   const client = DriveClient.launch({
      executable: DRIVE_FIXTURE,
      args: ['drive', 'settings.slab'],
   });

   deepEqual(await client.call('clock.advance', { ms: 24 }), { t: 24 });
   deepEqual(await client.quit(), { ok: true });
});

test('runs one standalone request through the dslab CLI', async () => {
   const stdout = capture();
   const stderr = capture();
   const code = await run(
      ['--slab', DRIVE_FIXTURE, 'settings.slab', 'clock.advance', '{"ms":24}'],
      stdout.output,
      stderr.output,
   );

   equal(code, 0, stderr.text());
   equal(stdout.text(), '{"t":24}\n');
   equal(stderr.text(), '');
});

test('connects CLI invocations to one persistent drive session', async () => {
   let t = 0;
   const server = await startServer((id, method, params, socket) => {
      if (method !== 'clock.advance' || !object(params) || typeof params.ms !== 'number') {
         socket.write(
            `${JSON.stringify({ id, error: { code: -32602, message: 'invalid request' } })}\n`,
         );
         return;
      }
      t += params.ms;
      socket.write(`${JSON.stringify({ id, result: { t } })}\n`);
   });
   const address = server.address();
   if (address === null || typeof address === 'string')
      throw new Error('test server has no TCP address');

   try {
      const first = capture();
      const second = capture();
      equal(
         await run(['--port', String(address.port), 'clock.advance', '{"ms":4}'], first.output),
         0,
      );
      equal(
         await run(['--port', String(address.port), 'clock.advance', '{"ms":6}'], second.output),
         0,
      );
      equal(first.text(), '{"t":4}\n');
      equal(second.text(), '{"t":10}\n');
   } finally {
      await stopServer(server);
   }
});

test('discovers verified native slab candidates in documented order', async () => {
   const attempted: string[] = [];
   const result = await discoverSlab(
      '/explicit/slab',
      { SLAB_BIN: '/env/slab', PATH: '' },
      '/home/tester',
      async (candidate) => {
         attempted.push(candidate);
         return candidate === '/home/tester/.cargo/bin/slab';
      },
   );
   equal(result, '/home/tester/.cargo/bin/slab');
   deepEqual(attempted, ['/explicit/slab', '/env/slab', '/home/tester/.cargo/bin/slab']);
});

test('reports every attempted slab path when none supports drive', async () => {
   const attempted: string[] = [];
   await rejects(
      discoverSlab(
         '/explicit/slab',
         { SLAB_BIN: '/env/slab', PATH: '' },
         '/home/tester',
         async (candidate) => {
            attempted.push(candidate);
            return false;
         },
      ),
      /attempted paths:\n {2}- \/explicit\/slab\n {2}- \/env\/slab\n {2}- \/home\/tester\/\.cargo\/bin\/slab/,
   );
   deepEqual(attempted, ['/explicit/slab', '/env/slab', '/home/tester/.cargo/bin/slab']);
});
