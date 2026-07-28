import Foundation

/// Resource classes referenced by compact GPU frame operations.
public enum SlabGPUResourceKind: UInt32, Sendable, Hashable {
    /// Linear, radial, or conic gradient ramp.
    case gradient = 0
    /// Static normalized vector path.
    case path = 1
    /// Font face bytes for glyph rasterization.
    case font = 2
    /// Compiled or runtime image pixels.
    case image = 3
    /// One box-shadow record.
    case shadow = 4
}

/// Identity and generation of one resource needed by a GPU frame.
public struct SlabGPUResourceRef: Sendable, Hashable {
    /// Resource class.
    public let kind: SlabGPUResourceKind
    /// Document-local resource index.
    public let index: UInt32
    /// Runtime generation; static resources use zero.
    public let generation: UInt32
}

/// A frame-local runtime path referenced by a negative Path operation index.
public struct SlabGPUPath: Sendable, Equatable {
    /// Absolute `M L C Q Z` verb codes.
    public let verbs: [UInt8]
    /// Coordinates consumed by the verb stream.
    public let coordinates: [Double]
}

/// One stop in a GPU gradient ramp.
public struct SlabGPUGradientStop: Sendable, Equatable {
    /// Normalized ramp position.
    public let position: Double
    /// Packed RGBA8 color.
    public let color: UInt32
}

/// A retained linear, radial, or conic gradient.
public struct SlabGPUGradient: Sendable, Equatable {
    /// Zero linear, one radial, or two conic.
    public let kind: UInt32
    /// Authored direction or start angle in degrees.
    public let angle: Double
    /// Ordered color stops.
    public let stops: [SlabGPUGradientStop]
}

/// Font bytes and selection metadata for native glyph rasterization.
public struct SlabGPUFont: Sendable, Equatable {
    /// Zero for proportional sans and one for monospace.
    public let familyClass: UInt32
    /// Selected CSS-compatible weight.
    public let weight: UInt32
    /// Authored font family.
    public let family: String
    /// OpenType font bytes.
    public let data: Data
}

/// Compiled PNG or runtime RGBA8 image data.
public struct SlabGPUImage: Sendable, Equatable {
    /// Pixel width.
    public let width: UInt32
    /// Pixel height.
    public let height: UInt32
    /// Zero PNG or one straight-alpha sRGB RGBA8.
    public let format: UInt32
    /// Encoded or raw image bytes according to `format`.
    public let data: Data
}

/// One retained box-shadow layer.
public struct SlabGPUShadow: Sendable, Equatable {
    /// Horizontal offset in layout points.
    public let x: Double
    /// Vertical offset in layout points.
    public let y: Double
    /// Blur radius in layout points.
    public let blur: Double
    /// Signed spread in layout points.
    public let spread: Double
    /// Packed RGBA8 color.
    public let color: UInt32
    /// Whether the shadow is inset.
    public let inset: Bool
}

/// One decoded resource payload returned by the binary GPU ABI.
public enum SlabGPUResource: Sendable, Equatable {
    /// Gradient resource.
    case gradient(SlabGPUGradient)
    /// Static path resource.
    case path(SlabGPUPath)
    /// Font resource.
    case font(SlabGPUFont)
    /// Image resource.
    case image(SlabGPUImage)
    /// Shadow resource.
    case shadow(SlabGPUShadow)
}

/// Compact frame streams consumed directly by retained native GPU buffers.
public struct SlabGPUFrame: Sendable, Equatable {
    /// Binary frame ABI revision.
    public let version: UInt32
    /// Document generation used to invalidate retained resource caches.
    public let documentGeneration: UInt32
    /// Operation tags and integer payload words.
    public let words: [UInt32]
    /// Frame width and height followed by operation float payloads.
    public let scalars: [Double]
    /// Frame-local text pool.
    public let strings: [String]
    /// Flat uncovered-glyph codepoint ranges.
    public let uncovered: [UInt32]
    /// Effects emitted while this frame settled retained gesture state.
    public let settledEffects: SlabEffects
    /// Frame-local runtime paths.
    public let runtimePaths: [SlabGPUPath]
    /// Diagnostics emitted by this solve.
    public let diagnostics: [SlabDiagnostic]
    /// Retained resources referenced by this frame.
    public let resources: [SlabGPUResourceRef]
    /// Whether retained state requires another settling frame.
    public let dirty: Bool
    /// Whether advancing the clock can change the next frame.
    public let motionActive: Bool

    /// Solved document width in layout points.
    public var width: Double { scalars[0] }
    /// Solved document height in layout points.
    public var height: Double { scalars[1] }

    init(packet: Data) throws {
        var reader = PacketReader(packet)
        try reader.expectMagic("SLFR")
        version = try reader.u32()
        guard version == 1 else {
            throw SlabRuntimeError.malformedResponse("unsupported GPU frame version \(version)")
        }
        let flags = try reader.u32()
        dirty = flags & 1 != 0
        motionActive = flags & 2 != 0
        documentGeneration = try reader.u32()
        let sectionCount = try reader.count()

        var words: [UInt32] = []
        var scalars: [Double] = []
        var strings: [String] = []
        var uncovered: [UInt32] = []
        var runtimePaths: [SlabGPUPath] = []
        var diagnostics: [SlabDiagnostic] = []
        var resources: [SlabGPUResourceRef] = []

        var settledEffects = SlabEffects(
            repaint: false,
            signals: [],
            caret: nil,
            inputMethod: nil,
            cursor: .arrow,
            focus: nil,
            scrolls: []
        )
        for _ in 0..<sectionCount {
            let kind = try reader.u32()
            var section = try reader.subreader(count: try reader.count())
            switch kind {
            case 1:
                words = try section.u32Array()
            case 2:
                scalars = try section.f64Array()
            case 3:
                strings = try section.stringArray()
            case 4:
                uncovered = try section.u32Array()
            case 5:
                let count = try section.count()
                runtimePaths.reserveCapacity(count)
                for _ in 0..<count {
                    let verbCount = try section.count()
                    let coordinateCount = try section.count()
                    let verbs = [UInt8](try section.data(count: verbCount))
                    var coordinates: [Double] = []
                    coordinates.reserveCapacity(coordinateCount)
                    for _ in 0..<coordinateCount {
                        coordinates.append(try section.f64())
                    }
                    runtimePaths.append(SlabGPUPath(verbs: verbs, coordinates: coordinates))
                }
            case 6:
                let count = try section.count()
                diagnostics.reserveCapacity(count)
                for _ in 0..<count {
                    let code = try section.string()
                    let line = try section.u32()
                    let message = try section.string()
                    diagnostics.append(
                        SlabDiagnostic(
                            level: "note",
                            code: code,
                            message: message,
                            line: Int(line),
                            remedy: nil
                        )
                    )
                }
            case 7:
                let count = try section.count()
                resources.reserveCapacity(count)
                for _ in 0..<count {
                    let rawKind = try section.u32()
                    guard let resourceKind = SlabGPUResourceKind(rawValue: rawKind) else {
                        throw SlabRuntimeError.malformedResponse("unknown GPU resource kind \(rawKind)")
                    }
                    resources.append(
                        SlabGPUResourceRef(
                            kind: resourceKind,
                            index: try section.u32(),
                            generation: try section.u32()
                        )
                    )
                }
            case 8:
                settledEffects = try section.effects()
            default:
                break
            }
            try section.finish()
        }
        try reader.finish()
        guard scalars.count >= 2 else {
            throw SlabRuntimeError.malformedResponse("GPU frame has no dimensions")
        }

        self.words = words
        self.scalars = scalars
        self.strings = strings
        self.uncovered = uncovered
        self.runtimePaths = runtimePaths
        self.diagnostics = diagnostics
        self.resources = resources
        self.settledEffects = settledEffects
    }
}

struct DecodedGPUResource {
    let kind: SlabGPUResourceKind
    let index: UInt32
    let generation: UInt32
    let value: SlabGPUResource

    init(packet: Data) throws {
        var reader = PacketReader(packet)
        try reader.expectMagic("SLRS")
        let version = try reader.u32()
        guard version == 1 else {
            throw SlabRuntimeError.malformedResponse("unsupported GPU resource version \(version)")
        }
        let rawKind = try reader.u32()
        guard let kind = SlabGPUResourceKind(rawValue: rawKind) else {
            throw SlabRuntimeError.malformedResponse("unknown GPU resource kind \(rawKind)")
        }
        self.kind = kind
        index = try reader.u32()
        generation = try reader.u32()

        switch kind {
        case .gradient:
            let gradientKind = try reader.u32()
            let angle = try reader.f64()
            let count = try reader.count()
            var stops: [SlabGPUGradientStop] = []
            stops.reserveCapacity(count)
            for _ in 0..<count {
                stops.append(
                    SlabGPUGradientStop(position: try reader.f64(), color: try reader.u32())
                )
            }
            value = .gradient(SlabGPUGradient(kind: gradientKind, angle: angle, stops: stops))
        case .path:
            let verbCount = try reader.count()
            let coordinateCount = try reader.count()
            let verbs = [UInt8](try reader.data(count: verbCount))
            var coordinates: [Double] = []
            coordinates.reserveCapacity(coordinateCount)
            for _ in 0..<coordinateCount {
                coordinates.append(try reader.f64())
            }
            value = .path(SlabGPUPath(verbs: verbs, coordinates: coordinates))
        case .font:
            let familyClass = try reader.u32()
            let weight = try reader.u32()
            let family = try reader.string()
            let bytes = try reader.data(count: try reader.count())
            value = .font(
                SlabGPUFont(familyClass: familyClass, weight: weight, family: family, data: bytes)
            )
        case .image:
            let width = try reader.u32()
            let height = try reader.u32()
            let format = try reader.u32()
            let bytes = try reader.data(count: try reader.count())
            value = .image(SlabGPUImage(width: width, height: height, format: format, data: bytes))
        case .shadow:
            value = .shadow(
                SlabGPUShadow(
                    x: try reader.f64(),
                    y: try reader.f64(),
                    blur: try reader.f64(),
                    spread: try reader.f64(),
                    color: try reader.u32(),
                    inset: try reader.u32() != 0
                )
            )
        }
        try reader.finish()
    }
}

private struct PacketReader {
    private let bytes: Data
    private var offset: Int
    private let limit: Int

    init(_ bytes: Data, offset: Int = 0, limit: Int? = nil) {
        self.bytes = bytes
        self.offset = offset
        self.limit = limit ?? bytes.count
    }

    mutating func expectMagic(_ expected: String) throws {
        let magic = try data(count: 4)
        guard magic == Data(expected.utf8) else {
            throw SlabRuntimeError.malformedResponse("GPU packet magic mismatch")
        }
    }

    mutating func u32() throws -> UInt32 {
        let value = try fixed(count: 4)
        return UInt32(value[0])
            | UInt32(value[1]) << 8
            | UInt32(value[2]) << 16
            | UInt32(value[3]) << 24
    }

    mutating func f64() throws -> Double {
        let value = try fixed(count: 8)
        var bits: UInt64 = 0
        for (shift, byte) in value.enumerated() {
            bits |= UInt64(byte) << UInt64(shift * 8)
        }
        return Double(bitPattern: bits)
    }

    mutating func count() throws -> Int {
        guard let count = Int(exactly: try u32()) else {
            throw SlabRuntimeError.memoryOutOfBounds
        }
        return count
    }

    mutating func string() throws -> String {
        let bytes = try data(count: count())
        guard let string = String(data: bytes, encoding: .utf8) else {
            throw SlabRuntimeError.malformedResponse("GPU packet string is not UTF-8")
        }
        return string
    }

    mutating func data(count: Int) throws -> Data {
        let range = try range(count: count)
        return bytes.subdata(in: range)
    }

    mutating func subreader(count: Int) throws -> PacketReader {
        let range = try range(count: count)
        return PacketReader(bytes, offset: range.lowerBound, limit: range.upperBound)
    }

    mutating func u32Array() throws -> [UInt32] {
        let count = try count()
        var values: [UInt32] = []
        values.reserveCapacity(count)
        for _ in 0..<count {
            values.append(try u32())
        }
        return values
    }

    mutating func f64Array() throws -> [Double] {
        let count = try count()
        var values: [Double] = []
        values.reserveCapacity(count)
        for _ in 0..<count {
            values.append(try f64())
        }
        return values
    }

    mutating func stringArray() throws -> [String] {
        let count = try count()
        var values: [String] = []
        values.reserveCapacity(count)
        for _ in 0..<count {
            values.append(try string())
        }
        return values
    }

    mutating func effects() throws -> SlabEffects {
        let flags = try u32()
        let cursor = SlabCursor(rawValue: try u32())
        let focusValue = try string()
        let caretRect = try rect()
        let inputMethodRect = try rect()
        let signalCount = try count()
        var signals: [SlabSignal] = []
        signals.reserveCapacity(signalCount)
        for _ in 0..<signalCount {
            let name = try string()
            let text = try string()
            let item = try string()
            let x = try f64()
            let y = try f64()
            let deltaX = try f64()
            let deltaY = try f64()
            let dragDeltaX = try f64()
            let dragDeltaY = try f64()
            let modifiers = try u32()
            let button = try u32()
            let clicks = try u32()
            let key = try string()
            let hitKey = try string()
            let pressedKey = try string()
            let sourceKey = try string()
            let sourceItem = try string()
            let cancelled = try u32() != 0
            let dropped = try u32() != 0
            signals.append(
                SlabSignal(
                    name: name,
                    text: text,
                    item: item,
                    meta: SlabSignalMeta(
                        x: x,
                        y: y,
                        deltaX: deltaX,
                        deltaY: deltaY,
                        dragDeltaX: dragDeltaX,
                        dragDeltaY: dragDeltaY,
                        modifiers: modifiers,
                        button: button,
                        clicks: clicks,
                        key: key,
                        hitKey: hitKey.isEmpty ? nil : hitKey,
                        pressedKey: pressedKey.isEmpty ? nil : pressedKey,
                        sourceKey: sourceKey,
                        sourceItem: sourceItem,
                        cancelled: cancelled,
                        dropped: dropped
                    )
                )
            )
        }
        let scrollCount = try count()
        var scrolls: [SlabScrollChange] = []
        scrolls.reserveCapacity(scrollCount)
        for _ in 0..<scrollCount {
            scrolls.append(
                SlabScrollChange(key: try string(), axis: try u32(), offset: try f64())
            )
        }
        return SlabEffects(
            repaint: flags & 1 != 0,
            signals: signals,
            caret: flags & 2 != 0 ? caretRect : nil,
            inputMethod: flags & 4 != 0 ? inputMethodRect : nil,
            cursor: cursor,
            focus: focusValue.isEmpty ? nil : focusValue,
            scrolls: scrolls
        )
    }

    private mutating func rect() throws -> SlabRect {
        SlabRect(x: try f64(), y: try f64(), width: try f64(), height: try f64())
    }

    mutating func finish() throws {
        guard offset == limit else {
            throw SlabRuntimeError.malformedResponse("GPU packet has \(limit - offset) trailing bytes")
        }
    }

    private mutating func fixed(count: Int) throws -> [UInt8] {
        let range = try range(count: count)
        return Array(bytes[range])
    }

    private mutating func range(count: Int) throws -> Range<Int> {
        guard count >= 0 else {
            throw SlabRuntimeError.memoryOutOfBounds
        }
        let end = offset.addingReportingOverflow(count)
        guard !end.overflow, end.partialValue <= limit else {
            throw SlabRuntimeError.memoryOutOfBounds
        }
        let range = offset..<end.partialValue
        offset = end.partialValue
        return range
    }
}
