# @stencil-hq/dslab

Typed client and CLI for the
[Slab Drive Protocol (SDP)](../../spec/SDP.md) — interrogate and drive a live
Slab kernel session from tests, agents, or scripts. SDP is newline-delimited
JSON over TCP or a spawned `slab drive` process.

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
dslab --port 4242 input.click '{"key":"#save"}'
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
await client.setFieldText('#search', 'Slab');
const text = await client.fieldText('#search');
const query = await client.param('query');
const focus = await client.focus();
await client.call('input.click', { key: '#toolbar/#save' });
const visible = await client.call('list.window', { each: '#feed/rows' });
await client.close();
```

### Wire tap

Every entry point (`connect`, `launch`, `fromStreams`) accepts an `onLine`
option that observes each NDJSON line on the wire — `send` lines as written,
`recv` lines as parsed, without the trailing newline. Use it for evidence-grade
transcripts instead of wrapping every call:

```ts
import { appendFileSync } from 'node:fs';

const client = await DriveClient.connect({
   port: 4242,
   onLine: (direction, line) => {
      appendFileSync('transcript.ndjson', `${direction} ${line}\n`);
   },
});
```

### Runtime diagnostics

`client.diags()` (SDP `doc.diags`) returns the cumulative runtime diagnostic
set — deduplicated `{code, line, msg}` entries ordered by first occurrence and
cleared only by a successful load — so one-shot per-solve notes such as
`glyph-missing` stay queryable after intermediate solves.

`DriveSceneKey` and `DriveEachKey` document the canonical locator surface.
Use an exact full key returned by `scene.tree`, a unique authored `#id`/`id`,
or a unique authored suffix rooted at an id such as `#toolbar/#save` or
`#feed/rows`. Component-call ids resolve to the expanded definition root.
Ambiguous locators fail with canonical candidate paths; unknown locators include
nearest/suffix suggestions.

In host-mounted `RequestPump` sessions, key/text input can pass through an
optional host callback and a consumed result has `host_consumed: true`. Host
reload is denied by default; an opted-in host must invalidate generated setter
caches before re-syncing. Drive signals and visible inputs rather than
`param.set` for host-owned values, because the next host sync overwrites those
params. Keep signal-bound UI affordances for host shortcuts that must remain
portable across hosts.

`DriveRemoteError` carries the structured SDP error code when the server
rejects a call. See the [normative SDP specification](../../spec/SDP.md) for
framing, determinism, every method, key grammar, host mounting, diagnostics,
and compatibility.
