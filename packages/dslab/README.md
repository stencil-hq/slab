# @stencil-hq/dslab

Typed client and CLI for the Slab Drive Protocol (SDP) — interrogate and drive
a live slab kernel session from tests, agents, or scripts. SDP is
newline-delimited JSON over TCP or a spawned `slab drive` process.

Requires Node ≥ 22 (or any recent Bun). Drive requires the native `slab-cli`:

```sh
cargo install --git https://github.com/stencil-hq/slab slab-cli
```

## CLI

```sh
# standalone: spawn `slab drive FILE` per invocation
dslab examples/10-settings.slab scene.find '{"text":"Save"}'

# connected: reuse a long-lived session
slab drive examples/10-settings.slab --port 4242
dslab --port 4242 input.click '{"key":"save"}'
dslab --port 4242 clock.advance '{"ms":25}'
```

```
dslab [--slab PATH] [--pretty] FILE METHOD [PARAMS]
dslab --port PORT [--host HOST] [--pretty] METHOD [PARAMS]
```

One result JSON value is printed per invocation; `--pretty` indents it.
Standalone discovery checks `--slab`, then `SLAB_BIN`, then an executable
`slab` on `PATH`, then `~/.cargo/bin/slab`. Each candidate must support
`slab drive --help`; failures list every attempted path.

## Library

```ts
import { DriveClient } from '@stencil-hq/dslab';

// connect to a running `slab drive FILE --port 4242`
const client = await DriveClient.connect({ port: 4242 });

// or own the process over stdio
const owned = DriveClient.launch({
   executable: 'slab',
   args: ['drive', 'doc.slab'],
});

const tree = await client.call('scene.tree');
await client.setFieldText('search', 'Slab');
const text = await client.fieldText('search');
const query = await client.param('query');
const focus = await client.focus();
await client.call('input.click', { key: 'save' });
await client.close();
```

`DriveRemoteError` carries the structured SDP error code when the server
rejects a call. See the package types for the full method/value surface.
