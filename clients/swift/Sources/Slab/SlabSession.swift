import Foundation

/// One live Slab document and its retained kernel state.
public actor SlabSession {
    private let runtime: SlabRuntime
    private let handle: UInt32
    private var nextRequestID: UInt64 = 0
    private var closed = false

    init(runtime: SlabRuntime, handle: UInt32) {
        self.runtime = runtime
        self.handle = handle
    }

    deinit {
        let runtime = runtime
        let handle = handle
        Task { await runtime.release(handle: handle) }
    }

    /// Releases this session; subsequent requests fail with `sessionClosed`.
    public func close() async {
        guard !closed else { return }
        closed = true
        await runtime.release(handle: handle)
    }

    /// Compiles inline `.slab` source and replaces the live document on success.
    public func open(source: String, name: String = "<source>") async throws {
        let response: LoadResponse = try await send(
            method: "doc.open",
            params: OpenParams(source: source, name: name)
        )
        guard response.ok else {
            throw SlabCompileError(name: name, diagnostics: response.diagnostics)
        }
    }

    /// Reads and compiles a host file without exposing a filesystem to WebAssembly.
    public func open(file url: URL) async throws {
        let source = try String(contentsOf: url, encoding: .utf8)
        try await open(source: source, name: url.path)
    }

    /// Returns the environment currently retained by the kernel.
    public func environment() async throws -> SlabEnvironment {
        try await send(method: "env.get", params: EmptyParams())
    }

    /// Atomically replaces the viewport and host traits used by the next solve.
    @discardableResult
    public func setEnvironment(_ environment: SlabEnvironment) async throws -> SlabEnvironment {
        try await send(method: "env.set", params: environment)
    }

    /// Renders the current frame as SVG for a native vector-image decoder.
    public func renderSVG() async throws -> SlabRenderFrame {
        let response: SVGResponse = try await send(method: "render.svg", params: EmptyParams())
        guard let svg = response.data?.data(using: .utf8) else {
            throw SlabRuntimeError.malformedResponse("render.svg did not return UTF-8 data")
        }
        guard svg.count == response.bytes else {
            throw SlabRuntimeError.malformedResponse(
                "render.svg reported \(response.bytes) bytes but returned \(svg.count)"
            )
        }
        return SlabRenderFrame(
            svg: svg,
            notes: response.notes,
            dirty: response.dirty ?? false,
            motionActive: response.motionActive ?? false
        )
    }
    /// Solves one compact binary GPU frame at an absolute motion timestamp.
    public func gpuFrame(atMilliseconds time: Double) async throws -> SlabGPUFrame {
        guard !closed else {
            throw SlabRuntimeError.sessionClosed
        }
        guard time.isFinite, time >= 0 else {
            throw SlabRuntimeError.invalidArgument("frame time must be finite and nonnegative")
        }
        return try SlabGPUFrame(packet: await runtime.frame(handle: handle, time: time))
    }

    /// Fetches one retained resource named by a compact GPU frame.
    public func gpuResource(_ reference: SlabGPUResourceRef) async throws -> SlabGPUResource {
        guard !closed else {
            throw SlabRuntimeError.sessionClosed
        }
        let packet = try await runtime.resource(
            handle: handle,
            kind: reference.kind.rawValue,
            index: reference.index
        )
        let resource = try DecodedGPUResource(packet: packet)
        guard resource.kind == reference.kind,
              resource.index == reference.index,
              resource.generation == reference.generation
        else {
            throw SlabRuntimeError.malformedResponse("GPU resource identity mismatch")
        }
        return resource.value
    }


    /// Advances the deterministic motion clock and returns its new millisecond value.
    @discardableResult
    public func advance(milliseconds: Double) async throws -> Double {
        guard milliseconds.isFinite, milliseconds >= 0 else {
            throw SlabRuntimeError.invalidArgument("clock delta must be finite and nonnegative")
        }
        let response: ClockResponse = try await send(
            method: "clock.advance",
            params: ClockParams(milliseconds: milliseconds)
        )
        return response.time
    }

    /// Dispatches one pointer move, press, or release through kernel hit testing.
    public func pointer(
        _ kind: SlabPointerKind,
        x: Double,
        y: Double,
        button: SlabPointerButton = .primary,
        clicks: UInt32 = 0,
        modifiers: [SlabModifier] = []
    ) async throws -> SlabEffects {
        let response: InputResponse = try await send(
            method: "input.pointer",
            params: PointerParams(
                kind: kind,
                x: x,
                y: y,
                button: button,
                clicks: clicks,
                modifiers: modifiers.isEmpty ? nil : modifiers
            )
        )
        return response.effects
    }

    /// Dispatches a two-axis wheel delta at one document position.
    public func wheel(
        x: Double,
        y: Double,
        deltaX: Double = 0,
        deltaY: Double,
        modifiers: [SlabModifier] = []
    ) async throws -> SlabEffects {
        let response: InputResponse = try await send(
            method: "input.wheel",
            params: WheelParams(
                x: x,
                y: y,
                deltaX: deltaX,
                deltaY: deltaY,
                modifiers: modifiers.isEmpty ? nil : modifiers
            )
        )
        return response.effects
    }

    /// Dispatches one platform-normalized key-down event.
    public func key(_ key: String, modifiers: [SlabModifier] = []) async throws -> SlabEffects {
        let response: InputResponse = try await send(
            method: "input.key",
            params: KeyParams(key: key, modifiers: modifiers.isEmpty ? nil : modifiers)
        )
        return response.effects
    }

    /// Dispatches committed text input independently from key-down handling.
    public func text(_ text: String) async throws -> SlabEffects {
        try await textDispatch(method: "input.text", text: text)
    }

    /// Dispatches host clipboard text as one paste operation.
    public func paste(_ text: String) async throws -> SlabEffects {
        try await textDispatch(method: "input.paste", text: text)
    }

    /// Starts an input-method composition in the focused editable field.
    public func compositionStarted() async throws -> SlabEffects {
        try await event(.compositionStart)
    }

    /// Replaces the active input-method composition text.
    public func compositionUpdated(_ text: String) async throws -> SlabEffects {
        try await event(.compositionUpdate, text: text)
    }

    /// Commits and ends the active input-method composition.
    public func compositionEnded(_ text: String) async throws -> SlabEffects {
        try await event(.compositionEnd, text: text)
    }

    /// Clears transient hover and pressed state when the native view loses focus.
    public func blur() async throws -> SlabEffects {
        try await event(.blur)
    }

    /// Calls an SDP method not yet covered by a typed convenience API.
    public func request<Parameters, Result>(
        method: String,
        params: Parameters,
        returning: Result.Type = Result.self
    ) async throws -> Result where Parameters: Encodable & Sendable, Result: Decodable & Sendable {
        try await send(method: method, params: params)
    }

    /// Calls a parameterless SDP method not yet covered by a typed convenience API.
    public func request<Result>(
        method: String,
        returning: Result.Type = Result.self
    ) async throws -> Result where Result: Decodable & Sendable {
        try await send(method: method, params: EmptyParams())
    }

    private func textDispatch(method: String, text: String) async throws -> SlabEffects {
        let response: InputResponse = try await send(method: method, params: TextParams(text: text))
        return response.effects
    }

    private func event(_ kind: EventKind, text: String? = nil) async throws -> SlabEffects {
        let response: InputResponse = try await send(
            method: "input.event",
            params: EventParams(kind: kind, text: text)
        )
        return response.effects
    }

    private func send<Parameters, Result>(
        method: String,
        params: Parameters
    ) async throws -> Result where Parameters: Encodable, Result: Decodable {
        guard !closed else {
            throw SlabRuntimeError.sessionClosed
        }
        nextRequestID &+= 1
        if nextRequestID == 0 {
            nextRequestID = 1
        }

        let request: WireRequest<Parameters>
        do {
            request = WireRequest(id: nextRequestID, method: method, params: params)
            let body = try JSONEncoder().encode(request)
            let bytes = try await runtime.request(handle: handle, body: body)
            let response = try JSONDecoder().decode(WireResponse<Result>.self, from: bytes)
            if let error = response.error {
                throw error
            }
            guard let result = response.result else {
                throw SlabRuntimeError.malformedResponse("\(method) returned neither result nor error")
            }
            return result
        } catch let error as SlabProtocolError {
            throw error
        } catch let error as SlabRuntimeError {
            throw error
        } catch {
            throw SlabRuntimeError.malformedResponse("\(method): \(error)")
        }
    }
}

private struct EmptyParams: Encodable, Sendable {}

private struct WireRequest<Parameters: Encodable>: Encodable {
    let id: UInt64
    let method: String
    let params: Parameters
}

private struct WireResponse<Result: Decodable>: Decodable {
    let result: Result?
    let error: SlabProtocolError?
}

private struct OpenParams: Encodable {
    let source: String
    let name: String
}

private struct LoadResponse: Decodable {
    let ok: Bool
    let diagnostics: [SlabDiagnostic]

    private enum CodingKeys: String, CodingKey {
        case ok
        case diagnostics = "diags"
    }
}

private struct SVGResponse: Decodable {
    let bytes: Int
    let notes: [String]
    let data: String?
    let dirty: Bool?
    let motionActive: Bool?

    private enum CodingKeys: String, CodingKey {
        case bytes
        case notes
        case data
        case dirty
        case motionActive = "motion_active"
    }
}

private struct ClockParams: Encodable {
    let milliseconds: Double

    private enum CodingKeys: String, CodingKey {
        case milliseconds = "ms"
    }
}

private struct ClockResponse: Decodable {
    let time: Double

    private enum CodingKeys: String, CodingKey {
        case time = "t"
    }
}

private struct PointerParams: Encodable {
    let kind: SlabPointerKind
    let x: Double
    let y: Double
    let button: SlabPointerButton
    let clicks: UInt32
    let modifiers: [SlabModifier]?

    private enum CodingKeys: String, CodingKey {
        case kind = "type"
        case x
        case y
        case button
        case clicks
        case modifiers = "mods"
    }
}

private struct WheelParams: Encodable {
    let x: Double
    let y: Double
    let deltaX: Double
    let deltaY: Double
    let modifiers: [SlabModifier]?

    private enum CodingKeys: String, CodingKey {
        case x
        case y
        case deltaX = "dx"
        case deltaY = "dy"
        case modifiers = "mods"
    }
}

private struct KeyParams: Encodable {
    let key: String
    let modifiers: [SlabModifier]?

    private enum CodingKeys: String, CodingKey {
        case key
        case modifiers = "mods"
    }
}

private struct TextParams: Encodable {
    let text: String
}

private enum EventKind: String, Encodable {
    case compositionStart = "composition-start"
    case compositionUpdate = "composition-update"
    case compositionEnd = "composition-end"
    case blur
}

private struct EventParams: Encodable {
    let kind: EventKind
    let text: String?

    private enum CodingKeys: String, CodingKey {
        case kind = "type"
        case text
    }
}

private struct InputResponse: Decodable {
    let effects: SlabEffects
    let time: Double

    private enum CodingKeys: String, CodingKey {
        case effects
        case time = "t"
    }
}
