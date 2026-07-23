import type { Writable } from 'node:stream';
type Output = Pick<Writable, 'write'>;
/** Runs the `dslab` command with injectable streams for embedding and tests. */
export declare function run(args: readonly string[], output?: Output, errors?: Output): Promise<number>;
export {};
