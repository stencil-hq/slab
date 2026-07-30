import { type ChildProcess, spawn } from 'node:child_process';
import { connect as connectSocket } from 'node:net';
import type { Readable, Writable } from 'node:stream';

/** A JSON value accepted and returned by the Slab Drive Protocol. */
export type DriveValue =
   | null
   | boolean
   | number
   | string
   | DriveValue[]
   | { [name: string]: DriveValue };

/** A renderer client name accepted by SDP environments. */
export type DriveClientKind = 'web' | 'gpu' | 'tui' | 'svg' | 'png';

/** A keyboard modifier accepted by SDP input methods. */
export type DriveModifier = 'shift' | 'alt' | 'ctrl' | 'meta';

/**
 * A canonical SDP scene-node locator.
 *
 * Use a full key returned by `scene.tree`, a unique `#id`/`id`, or a unique
 * authored suffix rooted at an id such as `#panel/#save`.
 */
export type DriveSceneKey = string;

/**
 * A canonical `each` locator accepted by list window, reveal, and focus APIs.
 *
 * Use the full `each` key, a unique `#id`/`id`, or an id-rooted suffix such as
 * `#feed/rows`.
 */
export type DriveEachKey = string;

/** A trace event name accepted by `input.event`. */
export type DriveEventType =
   | 'pointer-move'
   | 'pointer-down'
   | 'pointer-up'
   | 'wheel'
   | 'key-down'
   | 'text'
   | 'paste'
   | 'copy'
   | 'cut'
   | 'composition-start'
   | 'composition-update'
   | 'composition-end'
   | 'blur'
   | 'resize'
   | 'close'
   | 'inspect'
   | 'activate';

/** Direction of one NDJSON line observed by a DriveClient wire tap. */
export type DriveLineDirection = 'send' | 'recv';

/** Transport options accepted by every DriveClient entry point. */
export interface DriveClientOptions {
   /**
    * Observes every NDJSON line on the wire, without the trailing newline:
    * `send` lines as written, `recv` lines as parsed. Intended for
    * evidence-grade transcripts and debugging; throwing here is not caught.
    */
   onLine?: (direction: DriveLineDirection, line: string) => void;
}

/** One cumulative runtime diagnostic returned by `doc.diags`. */
export interface RuntimeDiagnostic {
   /** Stable diagnostic code, such as `glyph-missing`. */
   code: string;
   /** Source line, or 0 when the diagnostic has no authored anchor. */
   line: number;
   /** Human-readable message. */
   msg: string;
}

/** Connection details for an existing `slab drive --port` server. */
export interface DriveConnectOptions extends DriveClientOptions {
   /** TCP port printed by `slab drive --port`. */
   port: number;
   /** SDP host; defaults to the loopback address used by `slab drive`. */
   host?: string;
}

/** Process details for a `slab drive` stdio session owned by the client. */
export interface DriveLaunchOptions extends DriveClientOptions {
   /** Executable that starts the SDP server, usually `slab`. */
   executable: string;
   /** Arguments passed to the executable, usually `['drive', file]`. */
   args: readonly string[];
   /** Working directory for the spawned SDP server. */
   cwd?: string;
   /** Environment variables passed to the spawned SDP server. */
   env?: Record<string, string | undefined>;
}

/** Reports a structured error returned by the SDP server. */
export class DriveRemoteError extends Error {
   /** SDP error code returned by the server. */
   readonly code: number;

   /** SDP method that produced the error. */
   readonly method: string;

   constructor(method: string, code: number, message: string) {
      super(`${method}: ${message}`);
      this.name = 'DriveRemoteError';
      this.code = code;
      this.method = method;
   }
}

/** Reports malformed, unexpected, or unmatched data from an SDP transport. */
export class DriveProtocolError extends Error {
   constructor(message: string) {
      super(message);
      this.name = 'DriveProtocolError';
   }
}

/** Reports a request attempted after its SDP transport has closed. */
export class DriveClosedError extends Error {
   constructor(message = 'SDP transport is closed') {
      super(message);
      this.name = 'DriveClosedError';
   }
}

type DriveObject = { [name: string]: DriveValue };
type EmptyParams = Record<string, never>;
type Axis = 0 | 1;
type Rect = DriveObject & { x: number; y: number; w: number; h: number };
type Ok = DriveObject & { ok: true };
type Clock = DriveObject & { t: number };
type Environment = DriveObject & {
   width: number;
   height: number;
   client: DriveClientKind;
   dark: boolean;
   coarse: boolean;
   theme: string;
};
type Diagnostic = DriveObject & {
   level: string;
   code: string;
   msg: string;
   line: number;
   remedy?: string;
};
type DocumentParameter = DriveObject & {
   name: string;
   type: string;
   enum?: string[];
};
type SceneEntry = DriveObject & {
   i: number;
   node: number;
   key: DriveSceneKey;
   parent: number;
   kind: string;
   x: number;
   y: number;
   w: number;
   h: number;
   radius: number;
   rot: number;
   flags: number;
   clip: boolean;
   scroll: boolean;
   scroll_cross_enabled: boolean;
   inert: boolean;
   focusable: boolean;
   detached: boolean;
   scroll_off: number;
   content_main: number;
   scroll_cross: number;
   content_cross: number;
   role: string;
   label: string;
   desc: string;
   is_row: boolean;
};
type InputEffect = DriveObject & {
   repaint: boolean;
   signals: Array<
      DriveObject & {
         name: string;
         text: string;
         item: string;
         meta: DriveObject & {
            x: number;
            y: number;
            mods: number;
            button: number;
            clicks: number;
            key: string;
            hit_key?: DriveSceneKey;
            pressed_key?: string;
            src_key: DriveSceneKey;
            src_item: string;
         };
      }
   >;
   caret: Rect | null;
   ime: Rect | null;
   cursor: number;
   focus: string | null;
   scrolls: Array<DriveObject & { key: DriveSceneKey; axis: Axis; off: number }>;
   copy_text?: string;
   has_static_selection?: boolean;
};
type InputResult = DriveObject & { effects: InputEffect; t: number };
type TraceEvent = {
   type: DriveEventType;
   x?: number;
   y?: number;
   dx?: number;
   dy?: number;
   button?: number;
   clicks?: number;
   key?: string;
   text?: string;
   mods?: DriveModifier[];
};
type ImageInput =
   | {
        name: string;
        w: number;
        h: number;
        format: 1;
        rgba: number[];
        png_b64?: never;
     }
   | {
        name: string;
        w: number;
        h: number;
        format: 0;
        png_b64: string;
        rgba?: never;
     };
type ClickInput = {
   button?: number;
   clicks?: number;
   mods?: DriveModifier[];
} & ({ x: number; y: number; key?: never } | { key: DriveSceneKey; x?: never; y?: never });
type HoleSizeInput =
   | { name: string; hole?: never; w: number; h: number }
   | { hole: number; name?: never; w: number; h: number };
type ListFieldInput =
   | { param: string; path: string; index: number; field: string; kind: 'text'; value: string }
   | {
        param: string;
        path: string;
        index: number;
        field: string;
        kind: 'num' | 'pct';
        value: number;
     }
   | { param: string; path: string; index: number; field: string; kind: 'color'; value: number }
   | {
        param: string;
        path: string;
        index: number;
        field: string;
        kind: 'bool';
        value: boolean | number;
     }
   | { param: string; path: string; index: number; field: string; kind: 'enum'; value: string };
type ParameterInput =
   | { name: string; value: DriveValue; sets?: never }
   | { sets: Record<string, DriveValue>; name?: never; value?: never };
type Endpoint<Params extends object, Result extends DriveValue> = {
   params: Params;
   result: Result;
};

interface DriveApi {
   'protocol.info': Endpoint<
      EmptyParams,
      DriveObject & { name: 'sdp'; version: number; doc: string | null; methods: string[] }
   >;
   'protocol.quit': Endpoint<EmptyParams, Ok>;
   'doc.load': Endpoint<
      { file: string },
      DriveObject & { ok: boolean; diags: Diagnostic[]; reloaded?: true; theme_reset?: true }
   >;
   'doc.reload': Endpoint<
      EmptyParams,
      DriveObject & { ok: boolean; diags: Diagnostic[]; reloaded?: true; theme_reset?: true }
   >;
   'doc.info': Endpoint<
      EmptyParams,
      DriveObject & {
         file: string;
         params: DocumentParameter[];
         themes: string[];
         holes: string[];
         signals: string[];
         env: Environment;
         t: number;
      }
   >;
   'doc.diags': Endpoint<EmptyParams, DriveObject & { diags: RuntimeDiagnostic[] }>;
   'env.get': Endpoint<EmptyParams, Environment>;
   'env.set': Endpoint<Partial<Environment>, Environment>;
   'clock.get': Endpoint<EmptyParams, Clock>;
   'clock.advance': Endpoint<{ ms: number }, Clock>;
   'param.set': Endpoint<ParameterInput, Ok>;
   'param.get': Endpoint<{ name: string }, DriveObject & { value: DriveValue }>;
   'field.set': Endpoint<
      { key: DriveSceneKey; text: string },
      DriveObject & { ok: true; changed: boolean }
   >;
   'field.get': Endpoint<{ key: DriveSceneKey }, DriveObject & { text: string }>;
   'state.set': Endpoint<{ name: string; on: boolean }, Ok>;
   'state.node': Endpoint<{ key: DriveSceneKey; name: string; on: boolean }, Ok>;
   'focus.get': Endpoint<
      EmptyParams,
      DriveObject & { focus: number; key: DriveSceneKey; visible: boolean }
   >;
   'focus.set': Endpoint<{ key: DriveSceneKey; visible?: boolean }, Ok>;
   'img.register': Endpoint<ImageInput, DriveObject & { img: number }>;
   'img.unregister': Endpoint<{ name: string }, Ok>;
   'img.info': Endpoint<
      { img: number },
      DriveObject & { w: number; h: number; format: number; generation: number }
   >;
   'img.data': Endpoint<{ img: number }, DriveObject & { data: string; bytes: number }>;
   'scroll.get': Endpoint<
      { key: DriveSceneKey; axis: Axis },
      DriveObject & { axis: Axis; off: number }
   >;
   'scroll.set': Endpoint<
      { key: DriveSceneKey; axis: Axis; off: number },
      DriveObject & { axis: Axis; off: number }
   >;
   'scroll.reveal': Endpoint<{ key: DriveSceneKey; margin: number }, Ok>;
   'list.get_len': Endpoint<{ param: string; path: string }, DriveObject & { len: number }>;
   'list.set_len': Endpoint<{ param: string; path: string; n: number }, Ok>;
   'list.set_field': Endpoint<ListFieldInput, Ok>;
   'list.set_key': Endpoint<{ param: string; path: string; index: number; key: string }, Ok>;
   'list.reveal_item': Endpoint<{ each: DriveEachKey; index: number; align: 0 | 1 | 2 | 3 }, Ok>;
   'list.window': Endpoint<{ each: DriveEachKey }, DriveObject & { start: number; end: number }>;
   'divider.get': Endpoint<{ key: DriveSceneKey }, DriveObject & { extent: number }>;
   'divider.set': Endpoint<{ key: DriveSceneKey; extent: number }, Ok>;
   'split.get': Endpoint<{ key: DriveSceneKey }, DriveObject & { size: number }>;
   'split.set': Endpoint<{ key: DriveSceneKey; size: number }, Ok>;
   'hole.list': Endpoint<
      EmptyParams,
      DriveObject & {
         holes: Array<
            DriveObject & {
               hole: number;
               name: string;
               x: number;
               y: number;
               w: number;
               h: number;
               clip: boolean;
            }
         >;
      }
   >;
   'hole.size': Endpoint<HoleSizeInput, Ok>;
   'scene.tree': Endpoint<EmptyParams, DriveObject & { nodes: SceneEntry[] }>;
   'scene.node': Endpoint<
      { key: DriveSceneKey; states?: string[] },
      SceneEntry & { states: Record<string, boolean> }
   >;
   'scene.text': Endpoint<
      { key: DriveSceneKey },
      DriveObject & {
         text: string;
         runs: Array<DriveObject & { text: string; x: number; y: number }>;
      }
   >;
   'scene.hit': Endpoint<
      { x: number; y: number },
      DriveObject & { keys: DriveSceneKey[]; nodes: number[]; rects: Rect[] }
   >;
   'scene.find': Endpoint<
      { text: string },
      DriveObject & {
         matches: Array<
            DriveObject & { key: DriveSceneKey; node: number; text: string; rect: Rect }
         >;
      }
   >;
   'frame.dump': Endpoint<EmptyParams, DriveValue>;
   'frame.summary': Endpoint<
      EmptyParams,
      DriveObject & {
         focus: string | null;
         edits: Array<DriveObject & { name: string; text: string; item: string }>;
         scroll: Array<DriveObject & { key: DriveSceneKey; axis: Axis; off: number }>;
      }
   >;
   'input.event': Endpoint<TraceEvent, InputResult & { host_consumed?: true }>;
   'input.pointer': Endpoint<
      {
         type: 'move' | 'down' | 'up';
         x: number;
         y: number;
         button?: number;
         clicks?: number;
         mods?: DriveModifier[];
      },
      InputResult
   >;
   'input.click': Endpoint<ClickInput, InputResult>;
   'input.wheel': Endpoint<
      { x: number; y: number; dy: number; dx?: number; mods?: DriveModifier[] },
      InputResult
   >;
   'input.key': Endpoint<
      { key: string; mods?: DriveModifier[] },
      InputResult & { host_consumed?: true }
   >;
   'input.text': Endpoint<{ text: string }, InputResult & { host_consumed?: true }>;
   'input.paste': Endpoint<{ text: string }, InputResult & { host_consumed?: true }>;
   'render.png': Endpoint<
      { scale?: number; path?: string },
      DriveObject & {
         bytes: number;
         width_px: number;
         height_px: number;
         notes: string[];
         data?: string;
         path?: string;
      }
   >;
   'render.svg': Endpoint<
      { path?: string },
      DriveObject & { bytes: number; notes: string[]; data?: string; path?: string }
   >;
   'render.cells': Endpoint<
      { plain?: boolean; path?: string },
      DriveObject & {
         cols: number;
         rows: number;
         notes: string[];
         bytes?: number;
         text?: string;
         path?: string;
      }
   >;
   'render.apng': Endpoint<
      { dur?: number; fps?: number; scale?: number; path?: string },
      DriveObject & { bytes: number; frames: number; t: number; data?: string; path?: string }
   >;
}

/** Names every SDP method supported by `slab drive` protocol version 1. */
export type DriveMethod = keyof DriveApi;

/** Parameters accepted by one SDP method. */
export type DriveParams<Method extends DriveMethod> = DriveApi[Method]['params'];

/** Successful result emitted by one SDP method. */
export type DriveResult<Method extends DriveMethod> = DriveApi[Method]['result'];

type OptionalParamsMethod =
   | 'protocol.info'
   | 'protocol.quit'
   | 'doc.reload'
   | 'doc.info'
   | 'doc.diags'
   | 'env.get'
   | 'env.set'
   | 'clock.get'
   | 'focus.get'
   | 'hole.list'
   | 'scene.tree'
   | 'frame.dump'
   | 'frame.summary'
   | 'render.png'
   | 'render.svg'
   | 'render.cells'
   | 'render.apng';
type RequiredParamsMethod = Exclude<DriveMethod, OptionalParamsMethod>;
type Pending = {
   method: string;
   resolve(value: DriveValue): void;
   reject(error: Error): void;
};
type WireResponse =
   | { id: number; result: DriveValue }
   | { id: number; error: { code: number; message: string } };

function errorMessage(error: unknown): string {
   return error instanceof Error ? error.message : String(error);
}

function isObject(value: unknown): value is Record<string, unknown> {
   return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isDriveValue(value: unknown): value is DriveValue {
   if (value === null) return true;
   if (typeof value === 'boolean' || typeof value === 'number' || typeof value === 'string')
      return true;
   if (Array.isArray(value)) return value.every(isDriveValue);
   if (!isObject(value)) return false;
   for (const key in value) {
      if (!isDriveValue(value[key])) return false;
   }
   return true;
}

function parseResponse(line: string): WireResponse {
   let value: unknown;
   try {
      value = JSON.parse(line);
   } catch (error) {
      throw new DriveProtocolError(`SDP response is not valid JSON: ${errorMessage(error)}`);
   }
   if (!isObject(value)) {
      throw new DriveProtocolError('SDP response must be an object');
   }
   const id = value.id;
   if (typeof id !== 'number' || !Number.isSafeInteger(id)) {
      throw new DriveProtocolError('SDP response must include a numeric id');
   }
   if ('error' in value) {
      const error = value.error;
      if (!isObject(error) || typeof error.code !== 'number' || typeof error.message !== 'string') {
         throw new DriveProtocolError('SDP error response has an invalid error object');
      }
      return { id, error: { code: error.code, message: error.message } };
   }
   if (!('result' in value) || !isDriveValue(value.result)) {
      throw new DriveProtocolError('SDP success response has no JSON result');
   }
   return { id, result: value.result };
}

function stopChild(child: ChildProcess): Promise<void> {
   if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve();
   const { promise, resolve } = Promise.withResolvers<void>();
   const done = () => resolve();
   child.once('close', done);
   if (!child.kill() && (child.exitCode !== null || child.signalCode !== null)) {
      child.off('close', done);
      resolve();
   }
   return promise;
}

/** Speaks SDP over stdio, TCP, or custom newline-delimited streams. */
export class DriveClient {
   readonly #input: Readable;
   readonly #output: Writable;
   readonly #stop?: () => Promise<void>;
   readonly #onLine?: (direction: DriveLineDirection, line: string) => void;
   readonly #pending = new Map<number, Pending>();
   #buffer = '';
   #nextId = 1;
   #failure: Error | undefined;
   #closed = false;

   private constructor(
      input: Readable,
      output: Writable,
      options?: DriveClientOptions,
      stop?: () => Promise<void>,
   ) {
      this.#input = input;
      this.#output = output;
      this.#stop = stop;
      this.#onLine = options?.onLine;
      this.#input.setEncoding('utf8');
      this.#input.on('data', this.#onData);
      this.#input.once('end', this.#onEnd);
      this.#input.once('close', this.#onEnd);
      this.#input.once('error', this.#onError);
      this.#output.once('error', this.#onError);
   }

   /** Connects to the TCP listener started by `slab drive --port`. */
   static connect(options: DriveConnectOptions): Promise<DriveClient> {
      const connection = Promise.withResolvers<DriveClient>();
      const socket = connectSocket({ host: options.host ?? '127.0.0.1', port: options.port });
      const fail = (error: Error) => {
         socket.destroy();
         connection.reject(error);
      };
      socket.once('error', fail);
      socket.once('connect', () => {
         socket.off('error', fail);
         connection.resolve(new DriveClient(socket, socket, options));
      });
      return connection.promise;
   }

   /** Starts and owns an SDP process connected through its standard streams. */
   static launch(options: DriveLaunchOptions): DriveClient {
      const child = spawn(options.executable, options.args, {
         cwd: options.cwd,
         env: options.env,
         stdio: ['pipe', 'pipe', 'inherit'],
      });
      if (child.stdin === null || child.stdout === null) {
         child.kill();
         throw new DriveProtocolError('failed to open SDP process streams');
      }
      const client = new DriveClient(child.stdout, child.stdin, options, () => stopChild(child));
      child.once('error', (error) => client.#fail(error));
      return client;
   }

   /** Binds the client to custom newline-delimited SDP streams. */
   static fromStreams(
      input: Readable,
      output: Writable,
      options?: DriveClientOptions,
   ): DriveClient {
      return new DriveClient(input, output, options);
   }

   /** Invokes an SDP method whose parameters are optional. */
   call<Method extends OptionalParamsMethod>(
      method: Method,
      params?: DriveParams<Method>,
   ): Promise<DriveResult<Method>>;

   /** Invokes an SDP method that requires a parameter object. */
   call<Method extends RequiredParamsMethod>(
      method: Method,
      params: DriveParams<Method>,
   ): Promise<DriveResult<Method>>;

   call(method: DriveMethod, params: object = {}): Promise<DriveValue> {
      return this.request(method, params);
   }

   /** Invokes a runtime-selected SDP method and returns its raw JSON result. */
   request(method: string, params: object = {}): Promise<DriveValue> {
      if (this.#closed) return Promise.reject(new DriveClosedError());
      if (this.#failure) return Promise.reject(this.#failure);
      if (this.#nextId > Number.MAX_SAFE_INTEGER) {
         return Promise.reject(new DriveProtocolError('SDP request id space is exhausted'));
      }
      const id = this.#nextId;
      this.#nextId += 1;
      let line: string | undefined;
      try {
         line = JSON.stringify({ id, method, params });
      } catch (error) {
         return Promise.reject(
            new DriveProtocolError(`cannot encode SDP request: ${errorMessage(error)}`),
         );
      }
      if (line === undefined)
         return Promise.reject(new DriveProtocolError('cannot encode SDP request'));
      this.#onLine?.('send', line);
      const response = Promise.withResolvers<DriveValue>();
      this.#pending.set(id, { method, resolve: response.resolve, reject: response.reject });
      try {
         this.#output.write(`${line}\n`, 'utf8', (error) => {
            if (error) this.#fail(error);
         });
      } catch (error) {
         this.#fail(new DriveClosedError(errorMessage(error)));
      }
      return response.promise;
   }

   /** Replaces the retained edit text for one field key. */
   setFieldText(key: DriveSceneKey, text: string): Promise<DriveResult<'field.set'>> {
      return this.call('field.set', { key, text });
   }

   /** Reads the retained edit text for one field key. */
   async fieldText(key: DriveSceneKey): Promise<string> {
      return (await this.call('field.get', { key })).text;
   }

   /** Reads one live scalar or list parameter value. */
   async param(name: string): Promise<DriveValue> {
      return (await this.call('param.get', { name })).value;
   }

   /** Reads the current kernel focus index, key, and visibility. */
   focus(): Promise<DriveResult<'focus.get'>> {
      return this.call('focus.get');
   }

   /** Reads the cumulative runtime diagnostics for the loaded document. */
   async diags(): Promise<RuntimeDiagnostic[]> {
      return (await this.call('doc.diags')).diags;
   }

   /** Sends `protocol.quit`, then closes the local streams and owned process. */
   async quit(): Promise<DriveResult<'protocol.quit'>> {
      try {
         return await this.call('protocol.quit');
      } finally {
         await this.close();
      }
   }

   /** Closes only the local transport; use `quit` to stop the SDP session. */
   async close(): Promise<void> {
      if (this.#closed) return;
      this.#closed = true;
      this.#fail(new DriveClosedError('SDP client closed'));
      this.#input.destroy();
      this.#output.destroy();
      if (this.#stop) await this.#stop();
   }

   #onData = (chunk: string): void => {
      this.#buffer += chunk;
      let newline = this.#buffer.indexOf('\n');
      while (newline >= 0) {
         const line = this.#buffer.slice(0, newline).replace(/\r$/, '');
         this.#buffer = this.#buffer.slice(newline + 1);
         if (line.trim().length > 0) this.#handleLine(line);
         newline = this.#buffer.indexOf('\n');
      }
   };

   #onEnd = (): void => {
      if (!this.#closed) this.#fail(new DriveClosedError('SDP transport closed before responding'));
   };

   #onError = (error: Error): void => {
      if (!this.#closed) this.#fail(error);
   };

   #handleLine(line: string): void {
      this.#onLine?.('recv', line);
      let response: WireResponse;
      try {
         response = parseResponse(line);
      } catch (error) {
         this.#fail(error instanceof Error ? error : new DriveProtocolError(errorMessage(error)));
         return;
      }
      const pending = this.#pending.get(response.id);
      if (!pending) {
         this.#fail(
            new DriveProtocolError(`SDP response has no pending request for id ${response.id}`),
         );
         return;
      }
      this.#pending.delete(response.id);
      if ('error' in response) {
         pending.reject(
            new DriveRemoteError(pending.method, response.error.code, response.error.message),
         );
      } else {
         pending.resolve(response.result);
      }
   }

   #fail(error: Error): void {
      if (this.#failure) return;
      this.#failure = error;
      for (const pending of this.#pending.values()) pending.reject(error);
      this.#pending.clear();
   }
}
