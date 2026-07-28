#if canImport(AppKit)
import AppKit
import MetalKit
import Slab

/// A native AppKit surface backed by Slab's shared kernel and direct Metal painter.
@MainActor
public final class SlabView: NSView {
    /// The retained document session rendered by this view.
    public let session: SlabSession

    /// Receives authored signals in deterministic dispatch order.
    public var onSignals: (([SlabSignal]) -> Void)?
    /// Receives renderer and layout diagnostics after each changed frame.
    public var onDiagnostics: (([String]) -> Void)?
    /// Receives compilation, protocol, rendering, and input failures.
    public var onError: (((any Error)) -> Void)?

    private let metalView: MTKView
    private let renderer: MetalRenderer
    private var queueTail: Task<Void, Never>?
    private var renderRevision: UInt64 = 0
    private var renderScheduled = false
    private var environmentDirty = true
    private var documentLoaded = false
    private var documentStarted: TimeInterval = 0
    private var cursor = SlabCursor.arrow
    private var trackingArea: NSTrackingArea?
    private var inputMethodRect: SlabRect?
    private var markedText: String?
    private var markedSelection = NSRange(location: 0, length: 0)
    private var animationTimer: Timer?
    private var animationFramePending = false

    /// Creates a native Metal surface for an existing live session.
    public init(session: SlabSession) throws {
        self.session = session
        let metalView = MTKView(frame: .zero)
        self.metalView = metalView
        renderer = try MetalRenderer(view: metalView)
        super.init(frame: .zero)
        metalView.autoresizingMask = [.width, .height]
        metalView.frame = bounds
        addSubview(metalView)
        renderer.onError = { [weak self] error in self?.report(error) }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        nil
    }

    /// Uses top-left document coordinates, matching Slab input and frame geometry.
    public override var isFlipped: Bool { true }

    /// Accepts keyboard and input-method focus after pointer activation.
    public override var acceptsFirstResponder: Bool { true }
    /// Keeps native input on the kernel-owning view instead of its Metal layer.
    public override func hitTest(_ point: NSPoint) -> NSView? {
        bounds.contains(point) ? self : nil
    }


    /// Compiles source into this view's session and schedules its first frame.
    public func load(source: String, name: String = "<source>") {
        enqueue { [weak self] in
            guard let self else { return }
            documentLoaded = false
            try await session.open(source: source, name: name)
            documentStarted = ProcessInfo.processInfo.systemUptime
            documentLoaded = true
            environmentDirty = true
            invalidateFrame()
        }
    }

    /// Schedules a fresh solve without rebuilding the retained document.
    public func refresh() {
        invalidateFrame()
    }


    /// Tracks viewport changes through the same kernel environment as every host.
    public override func setFrameSize(_ newSize: NSSize) {
        let changed = frame.size != newSize
        super.setFrameSize(newSize)
        if changed {
            environmentDirty = true
            invalidateFrame()
        }
    }

    /// Starts rendering once the view has a window and backing coordinate space.
    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        window?.acceptsMouseMovedEvents = true
        if window == nil {
            stopAnimation()
        } else {
            environmentDirty = true
            invalidateFrame()
        }
    }

    /// Re-solves authored dark-mode conditions when AppKit appearance changes.
    public override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        environmentDirty = true
        invalidateFrame()
    }

    /// Maintains one in-visible-rect mouse tracker as the view moves or resizes.
    public override func updateTrackingAreas() {
        if let trackingArea {
            removeTrackingArea(trackingArea)
        }
        let area = NSTrackingArea(
            rect: .zero,
            options: [.activeInKeyWindow, .inVisibleRect, .mouseEnteredAndExited, .mouseMoved],
            owner: self,
            userInfo: nil
        )
        addTrackingArea(area)
        trackingArea = area
        super.updateTrackingAreas()
    }

    /// Installs the kernel-requested cursor for the complete native surface.
    public override func resetCursorRects() {
        addCursorRect(bounds, cursor: cursor.appKitCursor)
    }

    /// Gives pointer presses keyboard and input-method focus before dispatch.
    public override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        dispatchPointer(.down, event: event)
    }

    /// Dispatches a primary-button release.
    public override func mouseUp(with event: NSEvent) {
        dispatchPointer(.up, event: event)
    }

    /// Dispatches primary-button drag movement under kernel pointer capture.
    public override func mouseDragged(with event: NSEvent) {
        dispatchPointer(.move, event: event)
    }

    /// Gives secondary presses focus so context signals can target fields.
    public override func rightMouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        dispatchPointer(.down, event: event)
    }

    /// Dispatches a secondary-button release.
    public override func rightMouseUp(with event: NSEvent) {
        dispatchPointer(.up, event: event)
    }

    /// Dispatches secondary-button drag movement.
    public override func rightMouseDragged(with event: NSEvent) {
        dispatchPointer(.move, event: event)
    }

    /// Gives auxiliary-button presses focus before dispatch.
    public override func otherMouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        dispatchPointer(.down, event: event)
    }

    /// Dispatches an auxiliary-button release.
    public override func otherMouseUp(with event: NSEvent) {
        dispatchPointer(.up, event: event)
    }

    /// Dispatches auxiliary-button drag movement.
    public override func otherMouseDragged(with event: NSEvent) {
        dispatchPointer(.move, event: event)
    }

    /// Dispatches hover movement in document coordinates.
    public override func mouseMoved(with event: NSEvent) {
        dispatchPointer(.move, event: event)
    }

    /// Clears kernel hover by moving the pointer outside document geometry.
    public override func mouseExited(with event: NSEvent) {
        dispatch { session in
            try await session.pointer(.move, x: -1, y: -1)
        }
    }

    /// Converts AppKit pixel or line scrolling into layout-point wheel deltas.
    public override func scrollWheel(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        let multiplier = event.hasPreciseScrollingDeltas ? 1.0 : 40.0
        let deltaX = -event.scrollingDeltaX * multiplier
        let deltaY = -event.scrollingDeltaY * multiplier
        let modifiers = Self.modifiers(from: event.modifierFlags)
        dispatch { session in
            try await session.wheel(
                x: point.x,
                y: point.y,
                deltaX: deltaX,
                deltaY: deltaY,
                modifiers: modifiers
            )
        }
    }

    /// Sends normalized key-down state before AppKit produces text or commands.
    public override func keyDown(with event: NSEvent) {
        if markedText == nil, let key = Self.keyName(for: event) {
            let modifiers = Self.modifiers(from: event.modifierFlags)
            dispatch { session in
                try await session.key(key, modifiers: modifiers)
            }
        }
        interpretKeyEvents([event])
    }

    /// Clears transient kernel input state when AppKit removes first responder.
    public override func resignFirstResponder() -> Bool {
        let resigned = super.resignFirstResponder()
        markedText = nil
        dispatch { session in
            try await session.blur()
        }
        return resigned
    }

    /// Pastes the current string pasteboard as one kernel editing transaction.
    @objc public func paste(_ sender: Any?) {
        guard let text = NSPasteboard.general.string(forType: .string) else { return }
        dispatch { session in
            try await session.paste(text)
        }
    }

    private func enqueue(_ operation: @escaping @MainActor () async throws -> Void) {
        let previous = queueTail
        queueTail = Task { @MainActor [weak self] in
            _ = await previous?.result
            guard let self, !Task.isCancelled else { return }
            do {
                try await operation()
            } catch {
                report(error)
            }
        }
    }

    private func dispatch(
        _ operation: @escaping @Sendable (SlabSession) async throws -> SlabEffects
    ) {
        enqueue { [weak self, session] in
            guard let self else { return }
            let effects = try await operation(session)
            apply(effects)
        }
    }

    private func dispatchPointer(_ kind: SlabPointerKind, event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        let button = Self.button(from: event.buttonNumber)
        let clicks = UInt32(clamping: event.clickCount)
        let modifiers = Self.modifiers(from: event.modifierFlags)
        dispatch { session in
            try await session.pointer(
                kind,
                x: point.x,
                y: point.y,
                button: button,
                clicks: clicks,
                modifiers: modifiers
            )
        }
    }

    private func apply(_ effects: SlabEffects) {
        cursor = effects.cursor
        inputMethodRect = effects.inputMethod ?? effects.caret
        window?.invalidateCursorRects(for: self)
        inputContext?.invalidateCharacterCoordinates()
        if !effects.signals.isEmpty {
            onSignals?(effects.signals)
        }
        if effects.repaint {
            invalidateFrame()
        }
    }

    private func invalidateFrame() {
        guard documentLoaded else { return }
        renderRevision &+= 1
        if renderRevision == 0 {
            renderRevision = 1
        }
        scheduleFrameIfNeeded()
    }

    private func scheduleFrameIfNeeded() {
        guard documentLoaded,
              !renderScheduled,
              window != nil,
              bounds.width > 0,
              bounds.height > 0
        else { return }
        renderScheduled = true
        enqueue { [weak self] in
            guard let self else { return }
            let revision = renderRevision
            let updatesEnvironment = environmentDirty
            do {
                if updatesEnvironment {
                    let environment = SlabEnvironment(
                        width: bounds.width,
                        height: bounds.height,
                        client: .gpu,
                        dark: usesDarkAppearance
                    )
                    try await session.setEnvironment(environment)
                    environmentDirty = false
                }
                let milliseconds = max(
                    0,
                    (ProcessInfo.processInfo.systemUptime - documentStarted) * 1_000
                )
                let frame = try await session.gpuFrame(atMilliseconds: milliseconds)
                for reference in frame.resources
                    where renderer.needs(reference, document: frame.documentGeneration)
                {
                    let resource = try await session.gpuResource(reference)
                    try renderer.install(
                        resource,
                        for: reference,
                        document: frame.documentGeneration
                    )
                }
                try renderer.install(frame)
                finishFrame(frame, revision: revision)
            } catch {
                renderScheduled = false
                animationFramePending = false
                if updatesEnvironment {
                    environmentDirty = true
                }
                if revision != renderRevision {
                    scheduleFrameIfNeeded()
                }
                throw error
            }
        }
    }

    private func finishFrame(_ frame: SlabGPUFrame, revision: UInt64) {
        renderScheduled = false
        animationFramePending = false
        guard revision == renderRevision else {
            scheduleFrameIfNeeded()
            return
        }
        metalView.needsDisplay = true
        let notes = frame.diagnostics.map { diagnostic in
            diagnostic.line == 0
                ? "\(diagnostic.code): \(diagnostic.message)"
                : "\(diagnostic.code) line \(diagnostic.line): \(diagnostic.message)"
        }
        surface(notes)
        apply(frame.settledEffects)
        updateAnimation(active: frame.motionActive)
        if frame.dirty {
            invalidateFrame()
        }
    }

    private var usesDarkAppearance: Bool {
        effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
    }

    private func surface(_ notes: [String]) {
        guard !notes.isEmpty else { return }
        if let onDiagnostics {
            onDiagnostics(notes)
        } else {
            for note in notes {
                NSLog("Slab: %@", note)
            }
        }
    }

    private func report(_ error: any Error) {
        if let onError {
            onError(error)
        } else {
            NSLog("Slab: %@", error.localizedDescription)
        }
    }

    private func updateAnimation(active: Bool) {
        guard active else {
            stopAnimation()
            return
        }
        guard animationTimer == nil else { return }
        let timer = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.animationTick() }
        }
        RunLoop.main.add(timer, forMode: .common)
        animationTimer = timer
    }

    private func stopAnimation() {
        animationTimer?.invalidate()
        animationTimer = nil
        animationFramePending = false
    }

    private func animationTick() {
        guard !animationFramePending, window != nil else { return }
        animationFramePending = true
        invalidateFrame()
    }

    private static func modifiers(from flags: NSEvent.ModifierFlags) -> [SlabModifier] {
        var modifiers: [SlabModifier] = []
        if flags.contains(.shift) { modifiers.append(.shift) }
        if flags.contains(.option) { modifiers.append(.alt) }
        if flags.contains(.control) { modifiers.append(.control) }
        if flags.contains(.command) { modifiers.append(.command) }
        return modifiers
    }

    private static func button(from number: Int) -> SlabPointerButton {
        switch number {
        case 1: .secondary
        case 2: .middle
        default: .primary
        }
    }

    private static func keyName(for event: NSEvent) -> String? {
        switch event.keyCode {
        case 48: "Tab"
        case 36, 76: "Enter"
        case 51: "Backspace"
        case 117: "Delete"
        case 53: "Escape"
        case 114: "Insert"
        case 115: "Home"
        case 119: "End"
        case 116: "PageUp"
        case 121: "PageDown"
        case 123: "ArrowLeft"
        case 124: "ArrowRight"
        case 125: "ArrowDown"
        case 126: "ArrowUp"
        case 122: "F1"
        case 120: "F2"
        case 99: "F3"
        case 118: "F4"
        case 96: "F5"
        case 97: "F6"
        case 98: "F7"
        case 100: "F8"
        case 101: "F9"
        case 109: "F10"
        case 103: "F11"
        case 111: "F12"
        case 105: "F13"
        case 107: "F14"
        case 113: "F15"
        case 106: "F16"
        case 64: "F17"
        case 79: "F18"
        case 80: "F19"
        case 90: "F20"
        default:
            event.charactersIgnoringModifiers?.first.map(String.init)
        }
    }

    private static func plainText(from value: Any) -> String {
        if let text = value as? String {
            return text
        }
        if let attributed = value as? NSAttributedString {
            return attributed.string
        }
        return String(describing: value)
    }
}

extension SlabView: @preconcurrency NSTextInputClient {
    /// Dispatches committed AppKit text or completes the active composition.
    public func insertText(_ string: Any, replacementRange: NSRange) {
        let text = Self.plainText(from: string)
        if markedText != nil {
            markedText = nil
            dispatch { session in
                try await session.compositionEnded(text)
            }
        } else {
            dispatch { session in
                try await session.text(text)
            }
        }
    }

    /// Key commands were already normalized and sent by `keyDown(with:)`.
    public override func doCommand(by selector: Selector) {}

    /// Starts or updates an AppKit input-method composition.
    public func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        let text = Self.plainText(from: string)
        let startsComposition = markedText == nil
        markedText = text
        markedSelection = selectedRange
        if startsComposition {
            dispatch { session in
                try await session.compositionStarted()
            }
        }
        dispatch { session in
            try await session.compositionUpdated(text)
        }
    }

    /// Commits the current marked text and closes the kernel composition.
    public func unmarkText() {
        guard let text = markedText else { return }
        markedText = nil
        dispatch { session in
            try await session.compositionEnded(text)
        }
    }

    /// Reports whether AppKit currently owns marked composition text.
    public func hasMarkedText() -> Bool {
        markedText != nil
    }

    /// Exposes the marked range required by `NSTextInputClient`.
    public func markedRange() -> NSRange {
        guard let markedText else {
            return NSRange(location: NSNotFound, length: 0)
        }
        return NSRange(location: 0, length: (markedText as NSString).length)
    }

    /// Exposes AppKit's selection inside the current marked text.
    public func selectedRange() -> NSRange {
        markedText == nil ? NSRange(location: 0, length: 0) : markedSelection
    }

    /// Slab owns field text, so AppKit cannot directly read arbitrary ranges.
    public func attributedSubstring(
        forProposedRange range: NSRange,
        actualRange: NSRangePointer?
    ) -> NSAttributedString? {
        actualRange?.pointee = NSRange(location: NSNotFound, length: 0)
        return nil
    }

    /// Slab does not request host-specific marked-text attributes.
    public func validAttributesForMarkedText() -> [NSAttributedString.Key] {
        []
    }

    /// Positions the input-method candidate window at the kernel caret rectangle.
    public func firstRect(forCharacterRange range: NSRange, actualRange: NSRangePointer?) -> NSRect {
        actualRange?.pointee = range
        let documentRect: NSRect
        if let inputMethodRect {
            documentRect = NSRect(
                x: inputMethodRect.x,
                y: inputMethodRect.y,
                width: max(1, inputMethodRect.width),
                height: max(1, inputMethodRect.height)
            )
        } else {
            documentRect = NSRect(x: 0, y: 0, width: 1, height: 1)
        }
        let windowRect = convert(documentRect, to: nil)
        return window?.convertToScreen(windowRect) ?? windowRect
    }

    /// Returns the only stable fallback index when kernel text is host-opaque.
    public func characterIndex(for point: NSPoint) -> Int {
        0
    }
}

private extension SlabCursor {
    var appKitCursor: NSCursor {
        switch self {
        case .pointer: .pointingHand
        case .text: .iBeam
        case .columnResize: .resizeLeftRight
        case .rowResize: .resizeUpDown
        default: .arrow
        }
    }
}
#endif
