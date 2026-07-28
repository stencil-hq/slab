import Foundation

/// Rendering clients understood by Slab environment conditions.
public enum SlabClient: String, Codable, Sendable {
    /// Browser rendering semantics.
    case web
    /// GPU or native-window rendering semantics.
    case gpu
    /// Terminal-cell rendering semantics.
    case tui
    /// Static SVG rendering semantics.
    case svg
    /// Static PNG rendering semantics.
    case png
}

/// The viewport and host traits used to solve a Slab document.
public struct SlabEnvironment: Codable, Sendable {
    /// Viewport width in layout points.
    public let width: Double
    /// Viewport height in layout points.
    public let height: Double
    /// Host rendering class used by `when client` conditions.
    public let client: SlabClient
    /// Whether the host uses a dark appearance.
    public let dark: Bool
    /// Whether the primary pointer is coarse.
    public let coarse: Bool
    /// Authored theme name, or `nil` to retain the current theme.
    public let theme: String?

    /// Creates an environment for one host viewport.
    public init(
        width: Double,
        height: Double,
        client: SlabClient = .gpu,
        dark: Bool = false,
        coarse: Bool = false,
        theme: String? = nil
    ) {
        self.width = width
        self.height = height
        self.client = client
        self.dark = dark
        self.coarse = coarse
        self.theme = theme
    }
}

/// Modifier names accepted by Slab input events.
public enum SlabModifier: String, Codable, Sendable {
    /// Shift modifier.
    case shift
    /// Option or Alt modifier.
    case alt
    /// Control modifier.
    case control = "ctrl"
    /// Command, Super, or Meta modifier.
    case command = "meta"
}

/// Pointer event phases accepted by the shared kernel.
public enum SlabPointerKind: String, Codable, Sendable {
    /// Pointer movement without changing button state.
    case move
    /// Pointer-button press.
    case down
    /// Pointer-button release.
    case up
}

/// Platform-neutral pointer button codes.
public enum SlabPointerButton: UInt32, Codable, Sendable {
    /// Primary pointer button.
    case primary = 0
    /// Middle pointer button.
    case middle = 1
    /// Secondary pointer button.
    case secondary = 2
}

/// A cursor shape requested by the Slab kernel.
public struct SlabCursor: RawRepresentable, Codable, Sendable, Equatable {
    /// Kernel cursor identifier.
    public let rawValue: UInt32

    /// Default arrow cursor.
    public static let arrow = SlabCursor(rawValue: 0)
    /// Pointing-hand cursor.
    public static let pointer = SlabCursor(rawValue: 1)
    /// Text insertion cursor.
    public static let text = SlabCursor(rawValue: 2)
    /// Horizontal resize cursor.
    public static let columnResize = SlabCursor(rawValue: 3)
    /// Vertical resize cursor.
    public static let rowResize = SlabCursor(rawValue: 4)

    /// Preserves known and future kernel cursor identifiers.
    public init(rawValue: UInt32) {
        self.rawValue = rawValue
    }

    /// Decodes the protocol's integer cursor representation.
    public init(from decoder: any Decoder) throws {
        let container = try decoder.singleValueContainer()
        rawValue = try container.decode(UInt32.self)
    }

    /// Encodes the protocol's integer cursor representation.
    public func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(rawValue)
    }
}

/// A document-space rectangle in layout points.
public struct SlabRect: Codable, Sendable, Equatable {
    /// Left edge.
    public let x: Double
    /// Top edge.
    public let y: Double
    /// Width.
    public let width: Double
    /// Height.
    public let height: Double

    /// Creates a document-space rectangle.
    public init(x: Double, y: Double, width: Double, height: Double) {
        self.x = x
        self.y = y
        self.width = width
        self.height = height
    }

    /// Decodes SDP's `[x,y,w,h]` rectangle representation.
    public init(from decoder: any Decoder) throws {
        let values = try decoder.singleValueContainer().decode([Double].self)
        guard values.count == 4 else {
            throw DecodingError.dataCorrupted(
                .init(codingPath: decoder.codingPath, debugDescription: "Slab rect needs four values")
            )
        }
        x = values[0]
        y = values[1]
        width = values[2]
        height = values[3]
    }

    /// Encodes SDP's `[x,y,w,h]` rectangle representation.
    public func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode([x, y, width, height])
    }
}

/// One compiler or runtime diagnostic surfaced by Slab.
public struct SlabDiagnostic: Codable, Sendable, Equatable {
    /// Diagnostic severity such as `error`, `warn`, or `note`.
    public let level: String
    /// Stable diagnostic category.
    public let code: String
    /// Human-readable message.
    public let message: String
    /// One-based source line, or zero for a document-wide diagnostic.
    public let line: Int
    /// Optional authored remedy.
    public let remedy: String?

    private enum CodingKeys: String, CodingKey {
        case level
        case code
        case message = "msg"
        case line
        case remedy
    }
}

/// Pointer, keyboard, and drag context attached to one authored signal.
public struct SlabSignalMeta: Decodable, Sendable, Equatable {
    /// Pointer x coordinate, or `-1` for keyboard input.
    public let x: Double
    /// Pointer y coordinate, or `-1` for keyboard input.
    public let y: Double
    /// Event-local horizontal delta.
    public let deltaX: Double
    /// Event-local vertical delta.
    public let deltaY: Double
    /// Accumulated horizontal drag delta.
    public let dragDeltaX: Double
    /// Accumulated vertical drag delta.
    public let dragDeltaY: Double
    /// Kernel modifier bitset.
    public let modifiers: UInt32
    /// Pointer button code.
    public let button: UInt32
    /// Consecutive click count.
    public let clicks: UInt32
    /// Canonical key of the signal emitter.
    public let key: String
    /// Deepest hit target, when pointer-derived.
    public let hitKey: String?
    /// Fired keyboard key, when keyboard-derived.
    public let pressedKey: String?
    /// Canonical drag-source key.
    public let sourceKey: String
    /// Drag-source list item identifier.
    public let sourceItem: String
    /// Whether a gesture ended by cancellation.
    public let cancelled: Bool
    /// Whether a drag ended in a successful drop.
    public let dropped: Bool

    private enum CodingKeys: String, CodingKey {
        case x
        case y
        case deltaX = "dx"
        case deltaY = "dy"
        case dragDeltaX = "drag_dx"
        case dragDeltaY = "drag_dy"
        case modifiers = "mods"
        case button
        case clicks
        case key
        case hitKey = "hit_key"
        case pressedKey = "pressed_key"
        case sourceKey = "src_key"
        case sourceItem = "src_item"
        case cancelled
        case dropped
    }
}

/// One authored signal emitted by an input dispatch.
public struct SlabSignal: Decodable, Sendable, Equatable {
    /// Authored signal name.
    public let name: String
    /// Text payload for editing and text-bearing signals.
    public let text: String
    /// Innermost list item identifier.
    public let item: String
    /// Input metadata captured by the kernel.
    public let meta: SlabSignalMeta
}

/// One scroll offset changed by input dispatch.
public struct SlabScrollChange: Decodable, Sendable, Equatable {
    /// Canonical key of the scrolling node.
    public let key: String
    /// Zero for the main axis and one for the cross axis.
    public let axis: UInt32
    /// Resulting clamped offset.
    public let offset: Double

    private enum CodingKeys: String, CodingKey {
        case key
        case axis
        case offset = "off"
    }
}

/// All host-visible effects produced by one kernel input dispatch.
public struct SlabEffects: Decodable, Sendable, Equatable {
    /// Whether the document needs another rendered frame.
    public let repaint: Bool
    /// Authored signals in dispatch order.
    public let signals: [SlabSignal]
    /// Current text caret rectangle.
    public let caret: SlabRect?
    /// Current input-method candidate rectangle.
    public let inputMethod: SlabRect?
    /// Requested host cursor.
    public let cursor: SlabCursor
    /// Canonical focused-node key.
    public let focus: String?
    /// Scroll changes in dispatch order.
    public let scrolls: [SlabScrollChange]

    private enum CodingKeys: String, CodingKey {
        case repaint
        case signals
        case caret
        case inputMethod = "ime"
        case cursor
        case focus
        case scrolls
    }
}

/// One vector frame produced by the embedded Slab renderer.
public struct SlabRenderFrame: Sendable, Equatable {
    /// UTF-8 SVG bytes ready for a native image decoder.
    public let svg: Data
    /// Renderer and layout diagnostics for this solve.
    public let notes: [String]
    /// Whether another solve is needed to settle retained state.
    public let dirty: Bool
    /// Whether advancing the virtual clock can change the next frame.
    public let motionActive: Bool
}

/// An SDP error returned by the embedded Slab session.
public struct SlabProtocolError: Error, Decodable, Sendable, Equatable, LocalizedError {
    /// JSON-RPC-compatible SDP error code.
    public let code: Int
    /// Human-readable protocol failure.
    public let message: String

    /// Formats the protocol code and message.
    public var errorDescription: String? {
        "Slab protocol error \(code): \(message)"
    }
}

/// A source document that failed Slab compilation.
public struct SlabCompileError: Error, Sendable, Equatable, LocalizedError {
    /// Document label supplied during compilation.
    public let name: String
    /// Ordered compiler diagnostics.
    public let diagnostics: [SlabDiagnostic]

    /// Reports the first compiler error with its source line.
    public var errorDescription: String? {
        guard let diagnostic = diagnostics.first(where: { $0.level == "error" }) else {
            return "Slab could not compile \(name)"
        }
        return "Slab could not compile \(name):\(diagnostic.line): \(diagnostic.message) (\(diagnostic.code))"
    }
}

/// Failures at the Swift-to-WebAssembly ABI boundary.
public enum SlabRuntimeError: Error, Sendable, Equatable, LocalizedError {
    /// The generated ABI module is absent from the Swift package resources.
    case moduleMissing
    /// WasmKit could not parse or instantiate the bundled module.
    case invalidModule(String)
    /// The module does not export a required ABI symbol.
    case missingExport(String)
    /// The module and Swift host implement different ABI revisions.
    case incompatibleABI(actual: UInt32, expected: UInt32)
    /// A WebAssembly call trapped.
    case trap(String)
    /// The module could not reserve request or response memory.
    case allocationFailed(Int)
    /// An ABI pointer or length escaped linear-memory bounds.
    case memoryOutOfBounds
    /// The module returned a response outside the SDP envelope contract.
    case malformedResponse(String)
    /// A request targeted a closed Swift session.
    case sessionClosed
    /// A host supplied an invalid input value.
    case invalidArgument(String)

    /// Describes the failed ABI invariant.
    public var errorDescription: String? {
        switch self {
        case .moduleMissing:
            "Slab ABI module is missing; run `just abi-wasm`"
        case let .invalidModule(message):
            "Slab ABI module is invalid: \(message)"
        case let .missingExport(name):
            "Slab ABI module does not export \(name)"
        case let .incompatibleABI(actual, expected):
            "Slab ABI version is \(actual), expected \(expected)"
        case let .trap(message):
            "Slab WebAssembly trap: \(message)"
        case let .allocationFailed(size):
            "Slab could not allocate \(size) bytes in WebAssembly memory"
        case .memoryOutOfBounds:
            "Slab WebAssembly response is outside linear memory"
        case let .malformedResponse(message):
            "Slab returned a malformed response: \(message)"
        case .sessionClosed:
            "Slab session is closed"
        case let .invalidArgument(message):
            "Invalid Slab input: \(message)"
        }
    }
}
