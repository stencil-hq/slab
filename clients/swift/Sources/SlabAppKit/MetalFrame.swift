#if canImport(AppKit)
import Foundation
import Slab

struct MetalRectOp {
    let backgroundKind: UInt32
    let background: UInt32
    let strokeKind: UInt32
    let stroke: UInt32
    let strokeAlignment: UInt32
    let strokeSides: UInt32
    let dashed: Bool
    let shadowOffset: Int32
    let shadowCount: Int32
    let x: Double
    let y: Double
    let width: Double
    let height: Double
    let radius: Double
    let strokeWidth: Double
    let dashOn: Double
    let dashOff: Double
    let opacity: Double
    let smooth: Double
    let grainAmount: Double
    let grainSize: Double
}

struct MetalTextOp {
    let string: Int32
    let font: Int32
    let weight: UInt32
    let color: UInt32
    let colorKind: UInt32
    let strike: Bool
    let italic: Bool
    let underline: Bool
    let x: Double
    let baseline: Double
    let measuredWidth: Double
    let size: Double
    let tracking: Double
    let opacity: Double
    let gradientBox: SIMD4<Double>
    let underlineOffset: Double
    let underlineThickness: Double
}

struct MetalImageOp {
    let image: Int32
    let fit: UInt32
    let x: Double
    let y: Double
    let width: Double
    let height: Double
    let radius: Double
    let opacity: Double
    let smooth: Double
}

struct MetalPathOp {
    let path: Int32
    let backgroundKind: UInt32
    let background: UInt32
    let strokeKind: UInt32
    let stroke: UInt32
    let dashed: Bool
    let x: Double
    let y: Double
    let strokeWidth: Double
    let dashOn: Double
    let dashOff: Double
    let opacity: Double
}

struct MetalClipOp {
    let x: Double
    let y: Double
    let width: Double
    let height: Double
    let radius: Double
}

struct MetalGroupOp {
    let maskKind: UInt32
    let mask: UInt32
    let opacity: Double
    let blur: Double
    let maskBox: SIMD4<Double>
}

struct MetalRotateOp {
    let center: SIMD2<Double>
    let degrees: Double
}

struct MetalScaleOp {
    let center: SIMD2<Double>
    let scale: SIMD2<Double>
}

struct MetalTiltOp {
    let center: SIMD2<Double>
    let xDegrees: Double
    let yDegrees: Double
    let depth: Double
}

struct MetalBackdropOp {
    let maskKind: UInt32
    let mask: UInt32
    let rect: SIMD4<Double>
    let radius: Double
    let blur: Double
    let saturation: Double
    let brightness: Double
}

enum MetalFrameOp {
    case rect(MetalRectOp)
    case text(MetalTextOp)
    case image(MetalImageOp)
    case path(MetalPathOp)
    case clipPush(MetalClipOp)
    case clipPop
    case groupPush(MetalGroupOp)
    case groupPop
    case rotatePush(MetalRotateOp)
    case rotatePop
    case backdrop(MetalBackdropOp)
    case scalePush(MetalScaleOp)
    case scalePop
    case tiltPush(MetalTiltOp)
    case tiltPop
}

struct MetalFrame {
    let width: Double
    let height: Double
    let operations: [MetalFrameOp]

    init(_ frame: SlabGPUFrame) throws {
        var words = WordReader(frame.words)
        var scalars = ScalarReader(frame.scalars)
        width = try scalars.next("frame width")
        height = try scalars.next("frame height")
        var operations: [MetalFrameOp] = []
        operations.reserveCapacity(frame.words.count / 4)

        while !words.isAtEnd {
            let tag = try words.next("operation tag")
            switch tag {
            case 0:
                _ = try words.next("Rect.node")
                let backgroundKind = try words.next("Rect.bg_kind")
                let background = try words.next("Rect.bg")
                let strokeKind = try words.next("Rect.stroke_kind")
                let stroke = try words.next("Rect.stroke")
                let strokeAlignment = try words.next("Rect.stroke_align")
                let strokeSides = try words.next("Rect.stroke_sides")
                let dashed = try words.next("Rect.has_dash") != 0
                let shadowOffset = try words.signed("Rect.shadow_off")
                let shadowCount = try words.signed("Rect.shadow_len")
                operations.append(
                    .rect(
                        MetalRectOp(
                            backgroundKind: backgroundKind,
                            background: background,
                            strokeKind: strokeKind,
                            stroke: stroke,
                            strokeAlignment: strokeAlignment,
                            strokeSides: strokeSides,
                            dashed: dashed,
                            shadowOffset: shadowOffset,
                            shadowCount: shadowCount,
                            x: try scalars.next("Rect.x"),
                            y: try scalars.next("Rect.y"),
                            width: try scalars.next("Rect.w"),
                            height: try scalars.next("Rect.h"),
                            radius: try scalars.next("Rect.radius"),
                            strokeWidth: try scalars.next("Rect.stroke_w"),
                            dashOn: try scalars.next("Rect.dash_on"),
                            dashOff: try scalars.next("Rect.dash_off"),
                            opacity: try scalars.next("Rect.opacity"),
                            smooth: try scalars.next("Rect.smooth"),
                            grainAmount: try scalars.next("Rect.grain_amount"),
                            grainSize: try scalars.next("Rect.grain_size")
                        )
                    )
                )
            case 1:
                _ = try words.next("Text.node")
                let string = try words.signed("Text.str_ref")
                let font = try words.signed("Text.font")
                let weight = try words.next("Text.weight")
                let color = try words.next("Text.color")
                let colorKind = try words.next("Text.color_kind")
                let strike = try words.next("Text.strike") != 0
                _ = try words.signed("Text.uncov_off")
                _ = try words.next("Text.uncov_len")
                let italic = try words.next("Text.italic") != 0
                let underline = try words.next("Text.underline") != 0
                operations.append(
                    .text(
                        MetalTextOp(
                            string: string,
                            font: font,
                            weight: weight,
                            color: color,
                            colorKind: colorKind,
                            strike: strike,
                            italic: italic,
                            underline: underline,
                            x: try scalars.next("Text.x"),
                            baseline: try scalars.next("Text.y_baseline"),
                            measuredWidth: try scalars.next("Text.measured_w"),
                            size: try scalars.next("Text.size"),
                            tracking: try scalars.next("Text.tracking"),
                            opacity: try scalars.next("Text.opacity"),
                            gradientBox: SIMD4(
                                try scalars.next("Text.gx"),
                                try scalars.next("Text.gy"),
                                try scalars.next("Text.gw"),
                                try scalars.next("Text.gh")
                            ),
                            underlineOffset: try scalars.next("Text.underline_offset"),
                            underlineThickness: try scalars.next("Text.underline_thickness")
                        )
                    )
                )
            case 2:
                _ = try words.next("Image.node")
                let image = try words.signed("Image.img")
                let fit = try words.next("Image.fit")
                operations.append(
                    .image(
                        MetalImageOp(
                            image: image,
                            fit: fit,
                            x: try scalars.next("Image.x"),
                            y: try scalars.next("Image.y"),
                            width: try scalars.next("Image.w"),
                            height: try scalars.next("Image.h"),
                            radius: try scalars.next("Image.radius"),
                            opacity: try scalars.next("Image.opacity"),
                            smooth: try scalars.next("Image.smooth")
                        )
                    )
                )
            case 3:
                _ = try words.next("PathDraw.node")
                let path = try words.signed("PathDraw.path")
                let backgroundKind = try words.next("PathDraw.bg_kind")
                let background = try words.next("PathDraw.bg")
                let strokeKind = try words.next("PathDraw.stroke_kind")
                let stroke = try words.next("PathDraw.stroke")
                let dashed = try words.next("PathDraw.has_dash") != 0
                operations.append(
                    .path(
                        MetalPathOp(
                            path: path,
                            backgroundKind: backgroundKind,
                            background: background,
                            strokeKind: strokeKind,
                            stroke: stroke,
                            dashed: dashed,
                            x: try scalars.next("PathDraw.dx"),
                            y: try scalars.next("PathDraw.dy"),
                            strokeWidth: try scalars.next("PathDraw.stroke_w"),
                            dashOn: try scalars.next("PathDraw.dash_on"),
                            dashOff: try scalars.next("PathDraw.dash_off"),
                            opacity: try scalars.next("PathDraw.opacity")
                        )
                    )
                )
            case 4:
                operations.append(
                    .clipPush(
                        MetalClipOp(
                            x: try scalars.next("ClipPush.x"),
                            y: try scalars.next("ClipPush.y"),
                            width: try scalars.next("ClipPush.w"),
                            height: try scalars.next("ClipPush.h"),
                            radius: try scalars.next("ClipPush.radius")
                        )
                    )
                )
                _ = try scalars.next("ClipPush.smooth")
            case 5:
                operations.append(.clipPop)
            case 6:
                _ = try words.next("GroupPush.node")
                let maskKind = try words.next("GroupPush.mask_kind")
                let mask = try words.next("GroupPush.mask")
                operations.append(
                    .groupPush(
                        MetalGroupOp(
                            maskKind: maskKind,
                            mask: mask,
                            opacity: try scalars.next("GroupPush.opacity"),
                            blur: try scalars.next("GroupPush.blur"),
                            maskBox: SIMD4(
                                try scalars.next("GroupPush.mx"),
                                try scalars.next("GroupPush.my"),
                                try scalars.next("GroupPush.mw"),
                                try scalars.next("GroupPush.mh")
                            )
                        )
                    )
                )
            case 7:
                operations.append(.groupPop)
            case 8:
                operations.append(
                    .rotatePush(
                        MetalRotateOp(
                            center: SIMD2(
                                try scalars.next("RotatePush.cx"),
                                try scalars.next("RotatePush.cy")
                            ),
                            degrees: try scalars.next("RotatePush.deg")
                        )
                    )
                )
            case 9:
                operations.append(.rotatePop)
            case 10:
                let maskKind = try words.next("Backdrop.mask_kind")
                let mask = try words.next("Backdrop.mask")
                operations.append(
                    .backdrop(
                        MetalBackdropOp(
                            maskKind: maskKind,
                            mask: mask,
                            rect: SIMD4(
                                try scalars.next("Backdrop.x"),
                                try scalars.next("Backdrop.y"),
                                try scalars.next("Backdrop.w"),
                                try scalars.next("Backdrop.h")
                            ),
                            radius: try scalars.next("Backdrop.radius"),
                            blur: try scalars.next("Backdrop.blur"),
                            saturation: try scalars.next("Backdrop.saturate"),
                            brightness: try scalars.next("Backdrop.brightness")
                        )
                    )
                )
                _ = try scalars.next("Backdrop.smooth")
            case 11:
                operations.append(
                    .scalePush(
                        MetalScaleOp(
                            center: SIMD2(
                                try scalars.next("ScalePush.cx"),
                                try scalars.next("ScalePush.cy")
                            ),
                            scale: SIMD2(
                                try scalars.next("ScalePush.sx"),
                                try scalars.next("ScalePush.sy")
                            )
                        )
                    )
                )
            case 12:
                operations.append(.scalePop)
            case 13:
                operations.append(
                    .tiltPush(
                        MetalTiltOp(
                            center: SIMD2(
                                try scalars.next("TiltPush.cx"),
                                try scalars.next("TiltPush.cy")
                            ),
                            xDegrees: try scalars.next("TiltPush.rx"),
                            yDegrees: try scalars.next("TiltPush.ry"),
                            depth: try scalars.next("TiltPush.depth")
                        )
                    )
                )
            case 14:
                operations.append(.tiltPop)
            default:
                throw SlabRuntimeError.malformedResponse("unknown GPU operation tag \(tag)")
            }
        }
        guard scalars.isAtEnd else {
            throw SlabRuntimeError.malformedResponse("GPU frame has trailing scalar values")
        }
        self.operations = operations
    }
}

private struct WordReader {
    private let values: [UInt32]
    private var index = 0

    init(_ values: [UInt32]) {
        self.values = values
    }

    var isAtEnd: Bool { index == values.count }

    mutating func next(_ field: String) throws -> UInt32 {
        guard index < values.count else {
            throw SlabRuntimeError.malformedResponse("GPU frame ended at \(field)")
        }
        defer { index += 1 }
        return values[index]
    }

    mutating func signed(_ field: String) throws -> Int32 {
        Int32(bitPattern: try next(field))
    }
}

private struct ScalarReader {
    private let values: [Double]
    private var index = 0

    init(_ values: [Double]) {
        self.values = values
    }

    var isAtEnd: Bool { index == values.count }

    mutating func next(_ field: String) throws -> Double {
        guard index < values.count else {
            throw SlabRuntimeError.malformedResponse("GPU frame ended at \(field)")
        }
        defer { index += 1 }
        return values[index]
    }
}
#endif
