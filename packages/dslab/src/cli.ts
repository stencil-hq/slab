import type { Writable } from 'node:stream';
import { DriveClient, DriveRemoteError } from './index.js';

const USAGE = `\
usage:
  dslab [--slab PATH] [--pretty] FILE METHOD [PARAMS]
  dslab --port PORT [--host HOST] [--pretty] METHOD [PARAMS]

Run one Slab Drive Protocol request and print its result JSON.

options:
  --slab PATH   native Slab executable for standalone mode (default: slab)
  --port PORT   connect to an existing slab drive --port session
  --host HOST   SDP host with --port (default: 127.0.0.1)
  --pretty      indent result JSON

examples:
  dslab examples/10-settings.slab scene.find '{"text":"Save"}'
  slab drive examples/10-settings.slab --port 4242
  dslab --port 4242 input.click '{"key":"save"}'
`;

type Output = Pick<Writable, 'write'>;
type Command =
   | { kind: 'help' }
   | {
        kind: 'launch';
        executable: string;
        file: string;
        method: string;
        rawParams: string | undefined;
        pretty: boolean;
     }
   | {
        kind: 'connect';
        host: string;
        port: number;
        method: string;
        rawParams: string | undefined;
        pretty: boolean;
     };

class UsageError extends Error {}

function optionValue(args: readonly string[], index: number, option: string): string {
   const value = args[index + 1];
   if (value === undefined) throw new UsageError(`missing value for ${option}`);
   return value;
}

function parsePort(value: string): number {
   const port = Number(value);
   if (!Number.isInteger(port) || port < 1 || port > 65_535) {
      throw new UsageError(`invalid port '${value}'`);
   }
   return port;
}

function parseCommand(args: readonly string[]): Command {
   const positional: string[] = [];
   let executable: string | undefined;
   let host: string | undefined;
   let port: number | undefined;
   let pretty = false;

   for (let index = 0; index < args.length; index += 1) {
      const argument = args[index];
      if (argument === '--help' || argument === '-h' || argument === 'help')
         return { kind: 'help' };
      if (argument === '--slab') {
         executable = optionValue(args, index, argument);
         index += 1;
      } else if (argument === '--host') {
         host = optionValue(args, index, argument);
         if (host.length === 0) throw new UsageError('--host cannot be empty');
         index += 1;
      } else if (argument === '--port') {
         port = parsePort(optionValue(args, index, argument));
         index += 1;
      } else if (argument === '--pretty') {
         pretty = true;
      } else if (argument.startsWith('-')) {
         throw new UsageError(`unknown option '${argument}'`);
      } else {
         positional.push(argument);
      }
   }

   if (port !== undefined) {
      if (executable !== undefined) throw new UsageError('--slab cannot be used with --port');
      if (positional.length < 1 || positional.length > 2) {
         throw new UsageError('connected mode needs METHOD and optional PARAMS');
      }
      return {
         kind: 'connect',
         host: host ?? '127.0.0.1',
         port,
         method: positional[0],
         rawParams: positional[1],
         pretty,
      };
   }

   if (host !== undefined) throw new UsageError('--host requires --port');
   if (positional.length < 2 || positional.length > 3) {
      throw new UsageError('standalone mode needs FILE, METHOD, and optional PARAMS');
   }
   return {
      kind: 'launch',
      executable: executable ?? 'slab',
      file: positional[0],
      method: positional[1],
      rawParams: positional[2],
      pretty,
   };
}

function parseParams(raw: string | undefined): object {
   if (raw === undefined) return {};
   let value: unknown;
   try {
      value = JSON.parse(raw);
   } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      throw new UsageError(`PARAMS must be a JSON object: ${message}`);
   }
   if (typeof value !== 'object' || value === null || Array.isArray(value)) {
      throw new UsageError('PARAMS must be a JSON object');
   }
   return value;
}

/** Runs the `dslab` command with injectable streams for embedding and tests. */
export async function run(
   args: readonly string[],
   output: Output = process.stdout,
   errors: Output = process.stderr,
): Promise<number> {
   let command: Command;
   try {
      command = parseCommand(args);
   } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      errors.write(`error: ${message}\n${USAGE}`);
      return 2;
   }

   if (command.kind === 'help') {
      output.write(USAGE);
      return 0;
   }

   let params: object;
   try {
      params = parseParams(command.rawParams);
   } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      errors.write(`error: ${message}\n${USAGE}`);
      return 2;
   }

   let client: DriveClient | undefined;
   try {
      if (command.kind === 'connect') {
         client = await DriveClient.connect({ host: command.host, port: command.port });
      } else {
         client = DriveClient.launch({
            executable: command.executable,
            args: ['drive', command.file],
         });
      }
      const result = await client.request(command.method, params);
      output.write(`${JSON.stringify(result, null, command.pretty ? 2 : undefined)}\n`);
      if (command.kind === 'launch' && command.method !== 'protocol.quit') {
         await client.quit();
      } else {
         await client.close();
      }
      client = undefined;
      return 0;
   } catch (error) {
      const message =
         error instanceof DriveRemoteError
            ? `SDP ${error.code}: ${error.message}`
            : error instanceof Error
              ? error.message
              : String(error);
      errors.write(`error: ${message}\n`);
      return 1;
   } finally {
      if (client) await client.close();
   }
}
