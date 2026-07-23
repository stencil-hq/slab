import { spawn } from 'node:child_process';
import { connect as connectSocket } from 'node:net';
/** Reports a structured error returned by the SDP server. */
export class DriveRemoteError extends Error {
    /** SDP error code returned by the server. */
    code;
    /** SDP method that produced the error. */
    method;
    constructor(method, code, message) {
        super(`${method}: ${message}`);
        this.name = 'DriveRemoteError';
        this.code = code;
        this.method = method;
    }
}
/** Reports malformed, unexpected, or unmatched data from an SDP transport. */
export class DriveProtocolError extends Error {
    constructor(message) {
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
function errorMessage(error) {
    return error instanceof Error ? error.message : String(error);
}
function isObject(value) {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}
function isDriveValue(value) {
    if (value === null)
        return true;
    if (typeof value === 'boolean' || typeof value === 'number' || typeof value === 'string')
        return true;
    if (Array.isArray(value))
        return value.every(isDriveValue);
    if (!isObject(value))
        return false;
    for (const key in value) {
        if (!isDriveValue(value[key]))
            return false;
    }
    return true;
}
function parseResponse(line) {
    let value;
    try {
        value = JSON.parse(line);
    }
    catch (error) {
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
function stopChild(child) {
    if (child.exitCode !== null || child.signalCode !== null)
        return Promise.resolve();
    const { promise, resolve } = Promise.withResolvers();
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
    #input;
    #output;
    #stop;
    #pending = new Map();
    #buffer = '';
    #nextId = 1;
    #failure;
    #closed = false;
    constructor(input, output, stop) {
        this.#input = input;
        this.#output = output;
        this.#stop = stop;
        this.#input.setEncoding('utf8');
        this.#input.on('data', this.#onData);
        this.#input.once('end', this.#onEnd);
        this.#input.once('close', this.#onEnd);
        this.#input.once('error', this.#onError);
        this.#output.once('error', this.#onError);
    }
    /** Connects to the TCP listener started by `slab drive --port`. */
    static connect(options) {
        const connection = Promise.withResolvers();
        const socket = connectSocket({ host: options.host ?? '127.0.0.1', port: options.port });
        const fail = (error) => {
            socket.destroy();
            connection.reject(error);
        };
        socket.once('error', fail);
        socket.once('connect', () => {
            socket.off('error', fail);
            connection.resolve(new DriveClient(socket, socket));
        });
        return connection.promise;
    }
    /** Starts and owns an SDP process connected through its standard streams. */
    static launch(options) {
        const child = spawn(options.executable, options.args, {
            cwd: options.cwd,
            env: options.env,
            stdio: ['pipe', 'pipe', 'inherit'],
        });
        if (child.stdin === null || child.stdout === null) {
            child.kill();
            throw new DriveProtocolError('failed to open SDP process streams');
        }
        const client = new DriveClient(child.stdout, child.stdin, () => stopChild(child));
        child.once('error', (error) => client.#fail(error));
        return client;
    }
    /** Binds the client to custom newline-delimited SDP streams. */
    static fromStreams(input, output) {
        return new DriveClient(input, output);
    }
    call(method, params = {}) {
        return this.request(method, params);
    }
    /** Invokes a runtime-selected SDP method and returns its raw JSON result. */
    request(method, params = {}) {
        if (this.#closed)
            return Promise.reject(new DriveClosedError());
        if (this.#failure)
            return Promise.reject(this.#failure);
        if (this.#nextId > Number.MAX_SAFE_INTEGER) {
            return Promise.reject(new DriveProtocolError('SDP request id space is exhausted'));
        }
        const id = this.#nextId;
        this.#nextId += 1;
        let line;
        try {
            line = JSON.stringify({ id, method, params });
        }
        catch (error) {
            return Promise.reject(new DriveProtocolError(`cannot encode SDP request: ${errorMessage(error)}`));
        }
        if (line === undefined)
            return Promise.reject(new DriveProtocolError('cannot encode SDP request'));
        const response = Promise.withResolvers();
        this.#pending.set(id, { method, resolve: response.resolve, reject: response.reject });
        try {
            this.#output.write(`${line}\n`, 'utf8', (error) => {
                if (error)
                    this.#fail(error);
            });
        }
        catch (error) {
            this.#fail(new DriveClosedError(errorMessage(error)));
        }
        return response.promise;
    }
    /** Sends `protocol.quit`, then closes the local streams and owned process. */
    async quit() {
        try {
            return await this.call('protocol.quit');
        }
        finally {
            await this.close();
        }
    }
    /** Closes only the local transport; use `quit` to stop the SDP session. */
    async close() {
        if (this.#closed)
            return;
        this.#closed = true;
        this.#fail(new DriveClosedError('SDP client closed'));
        this.#input.destroy();
        this.#output.destroy();
        if (this.#stop)
            await this.#stop();
    }
    #onData = (chunk) => {
        this.#buffer += chunk;
        let newline = this.#buffer.indexOf('\n');
        while (newline >= 0) {
            const line = this.#buffer.slice(0, newline).replace(/\r$/, '');
            this.#buffer = this.#buffer.slice(newline + 1);
            if (line.trim().length > 0)
                this.#handleLine(line);
            newline = this.#buffer.indexOf('\n');
        }
    };
    #onEnd = () => {
        if (!this.#closed)
            this.#fail(new DriveClosedError('SDP transport closed before responding'));
    };
    #onError = (error) => {
        if (!this.#closed)
            this.#fail(error);
    };
    #handleLine(line) {
        let response;
        try {
            response = parseResponse(line);
        }
        catch (error) {
            this.#fail(error instanceof Error ? error : new DriveProtocolError(errorMessage(error)));
            return;
        }
        const pending = this.#pending.get(response.id);
        if (!pending) {
            this.#fail(new DriveProtocolError(`SDP response has no pending request for id ${response.id}`));
            return;
        }
        this.#pending.delete(response.id);
        if ('error' in response) {
            pending.reject(new DriveRemoteError(pending.method, response.error.code, response.error.message));
        }
        else {
            pending.resolve(response.result);
        }
    }
    #fail(error) {
        if (this.#failure)
            return;
        this.#failure = error;
        for (const pending of this.#pending.values())
            pending.reject(error);
        this.#pending.clear();
    }
}
