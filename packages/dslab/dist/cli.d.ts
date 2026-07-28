import type { Writable } from 'node:stream';
type Output = Pick<Writable, 'write'>;
type Probe = (executable: string) => Promise<boolean>;
/** Finds a native Slab executable and verifies its `drive` command. */
export declare function discoverSlab(explicit: string | undefined, environment?: NodeJS.ProcessEnv, home?: string, probe?: Probe): Promise<string>;
/** Runs the `dslab` command with injectable streams for embedding and tests. */
export declare function run(args: readonly string[], output?: Output, errors?: Output): Promise<number>;
export {};
