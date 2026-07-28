import Foundation
import WasmKit

/// Owns one import-free Slab WebAssembly instance and serializes its sessions.
public actor SlabRuntime {
    /// C ABI revision implemented by this package.
    public static let abiVersion: UInt32 = 1

    private let core: RuntimeCore

    /// Loads the bundled compiler, kernel, renderer, and SDP session layer.
    public init() throws {
        guard let moduleURL = Bundle.module.url(forResource: "slab_abi", withExtension: "wasm") else {
            throw SlabRuntimeError.moduleMissing
        }
        let moduleData = try Data(contentsOf: moduleURL, options: .mappedIfSafe)
        core = try RuntimeCore(moduleData: moduleData, expectedVersion: Self.abiVersion)
    }

    /// Creates an independent live document session in this runtime.
    public func makeSession() throws -> SlabSession {
        let handle = try core.callOne(core.exports.sessionNew)
        guard handle != 0 else {
            throw SlabRuntimeError.allocationFailed(0)
        }
        return SlabSession(runtime: self, handle: handle)
    }

    func request(handle: UInt32, body: Data) throws -> Data {
        guard let length = UInt32(exactly: body.count), length > 0 else {
            throw SlabRuntimeError.invalidArgument("request must contain 1 through \(UInt32.max) bytes")
        }

        let requestPointer = try core.callOne(core.exports.allocate, length)
        guard requestPointer != 0 else {
            throw SlabRuntimeError.allocationFailed(body.count)
        }
        defer { core.free(pointer: requestPointer, length: length) }

        try core.checkRange(pointer: requestPointer, count: body.count)
        core.memory.withUnsafeMutableBufferPointer(
            offset: UInt(requestPointer),
            count: body.count
        ) { destination in
            body.withUnsafeBytes { source in
                destination.copyMemory(from: source)
            }
        }

        let responsePointer = try core.callOne(core.exports.request, handle, requestPointer, length)
        return try core.copyBlock(pointer: responsePointer, context: "slab_request")
    }

    func frame(handle: UInt32, time: Double) throws -> Data {
        let pointer = try core.callFrame(core.exports.frame, handle: handle, time: time)
        return try core.copyBlock(pointer: pointer, context: "slab_frame")
    }

    func resource(handle: UInt32, kind: UInt32, index: UInt32) throws -> Data {
        let pointer = try core.callOne(core.exports.resource, handle, kind, index)
        return try core.copyBlock(pointer: pointer, context: "slab_resource")
    }

    func release(handle: UInt32) {
        core.callIgnoringResult(core.exports.sessionFree, handle)
    }
}

private final class RuntimeCore {
    let engine: Engine
    let store: Store
    let module: Module
    let instance: Instance
    let memory: Memory
    let exports: ABIExports

    init(moduleData: Data, expectedVersion: UInt32) throws {
        do {
            let parsedModule = try parseWasm(bytes: Array(moduleData))
            let engine = Engine()
            let store = Store(engine: engine)
            let instance = try parsedModule.instantiate(store: store)
            guard let memory = instance.exports[memory: "memory"] else {
                throw SlabRuntimeError.missingExport("memory")
            }
            let exports = try ABIExports(instance: instance)
            let version = try Self.invokeOne(exports.version)
            guard version == expectedVersion else {
                throw SlabRuntimeError.incompatibleABI(actual: version, expected: expectedVersion)
            }

            self.engine = engine
            self.store = store
            module = parsedModule
            self.instance = instance
            self.memory = memory
            self.exports = exports
        } catch let error as SlabRuntimeError {
            throw error
        } catch {
            throw SlabRuntimeError.invalidModule(String(describing: error))
        }
    }

    func callOne(_ function: Function, _ arguments: UInt32...) throws -> UInt32 {
        do {
            return try Self.invokeOne(function, arguments)
        } catch let error as SlabRuntimeError {
            throw error
        } catch {
            throw SlabRuntimeError.trap(String(describing: error))
        }
    }
    func callFrame(_ function: Function, handle: UInt32, time: Double) throws -> UInt32 {
        do {
            return try Self.invokeOne(
                function,
                values: [.i32(handle), .f64(time.bitPattern)]
            )
        } catch let error as SlabRuntimeError {
            throw error
        } catch {
            throw SlabRuntimeError.trap(String(describing: error))
        }
    }

    func callIgnoringResult(_ function: Function, _ arguments: UInt32...) {
        let values = arguments.map { Value(signed: Int32(bitPattern: $0)) }
        _ = try? function(values)
    }

    func free(pointer: UInt32, length: UInt32) {
        guard pointer != 0 else { return }
        callIgnoringResult(exports.free, pointer, length)
    }

    func copyBlock(pointer: UInt32, context: String) throws -> Data {
        guard pointer != 0 else {
            throw SlabRuntimeError.malformedResponse("\\(context) returned a null block")
        }
        try checkRange(pointer: pointer, count: 4)
        let header = memory.withUnsafeMutableBufferPointer(offset: UInt(pointer), count: 4) {
            Array($0.bindMemory(to: UInt8.self))
        }
        let payloadLength = UInt32(header[0])
            | UInt32(header[1]) << 8
            | UInt32(header[2]) << 16
            | UInt32(header[3]) << 24
        let totalLength = payloadLength.addingReportingOverflow(4)
        guard !totalLength.overflow else {
            throw SlabRuntimeError.memoryOutOfBounds
        }
        defer { free(pointer: pointer, length: totalLength.partialValue) }
        guard let payloadCount = Int(exactly: payloadLength) else {
            throw SlabRuntimeError.memoryOutOfBounds
        }
        let payloadPointer = pointer.addingReportingOverflow(4)
        guard !payloadPointer.overflow else {
            throw SlabRuntimeError.memoryOutOfBounds
        }
        try checkRange(pointer: payloadPointer.partialValue, count: payloadCount)
        return memory.withUnsafeMutableBufferPointer(
            offset: UInt(payloadPointer.partialValue),
            count: payloadCount
        ) { Data($0) }
    }

    func checkRange(pointer: UInt32, count: Int) throws {
        guard count >= 0 else {
            throw SlabRuntimeError.memoryOutOfBounds
        }
        let start = Int(pointer)
        let end = start.addingReportingOverflow(count)
        guard !end.overflow, end.partialValue <= memory.data.count else {
            throw SlabRuntimeError.memoryOutOfBounds
        }
    }
    private static func invokeOne(_ function: Function, values: [Value]) throws -> UInt32 {
        let results = try function(values)
        guard results.count == 1 else {
            throw SlabRuntimeError.malformedResponse(
                "ABI export returned \\(results.count) values instead of one"
            )
        }
        return results[0].i32
    }

    private static func invokeOne(_ function: Function, _ arguments: [UInt32] = []) throws -> UInt32 {
        try invokeOne(
            function,
            values: arguments.map { Value(signed: Int32(bitPattern: $0)) }
        )
    }
}

private struct ABIExports {
    let version: Function
    let allocate: Function
    let free: Function
    let sessionNew: Function
    let sessionFree: Function
    let sessionQuit: Function
    let request: Function
    let frame: Function
    let resource: Function

    init(instance: Instance) throws {
        version = try Self.function(named: "slab_abi_version", in: instance)
        allocate = try Self.function(named: "slab_alloc", in: instance)
        free = try Self.function(named: "slab_free", in: instance)
        sessionNew = try Self.function(named: "slab_session_new", in: instance)
        sessionFree = try Self.function(named: "slab_session_free", in: instance)
        sessionQuit = try Self.function(named: "slab_session_quit", in: instance)
        request = try Self.function(named: "slab_request", in: instance)
        frame = try Self.function(named: "slab_frame", in: instance)
        resource = try Self.function(named: "slab_resource", in: instance)
    }

    private static func function(named name: String, in instance: Instance) throws -> Function {
        guard let function = instance.exports[function: name] else {
            throw SlabRuntimeError.missingExport(name)
        }
        return function
    }
}
