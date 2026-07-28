#if canImport(AppKit)
import AppKit
import CoreText
import ImageIO
import Metal
import MetalKit
import MetalPerformanceShaders
import Slab
import simd

@MainActor
final class MetalRenderer: NSObject, MTKViewDelegate {
    var onError: ((any Error) -> Void)?

    private let device: any MTLDevice
    private let queue: any MTLCommandQueue
    private let pipeline: any MTLRenderPipelineState
    private let sampler: any MTLSamplerState
    private let textureLoader: MTKTextureLoader
    private let dummyTexture: any MTLTexture
    private var presentation: MetalPresentation?
    private var documentGeneration: UInt32 = 0
    private var resources: [ResourceAddress: MetalResource] = [:]
    private var resourceVersions: [ResourceAddress: UInt32] = [:]
    private var stagedDocumentGeneration: UInt32?
    private var stagedResources: [ResourceAddress: MetalResource] = [:]
    private var stagedResourceVersions: [ResourceAddress: UInt32] = [:]
    private var pathMasks: [PathMaskKey: PathMask] = [:]
    private var textMasks: [TextMaskKey: TextMask] = [:]
    private var sceneTexture: (any MTLTexture)?
    private var layerTextures: [any MTLTexture] = []
    private var auxiliaryTextures: [any MTLTexture] = []
    private var explicitDraw = false
    private var drawSucceeded = false

    init(view: MTKView) throws {
        guard let device = view.device ?? MTLCreateSystemDefaultDevice() else {
            throw SlabRuntimeError.invalidArgument("Metal is unavailable on this Mac")
        }
        guard let queue = device.makeCommandQueue() else {
            throw SlabRuntimeError.invalidArgument("Metal could not create a command queue")
        }
        let library = try device.makeLibrary(source: metalShader, options: nil)
        guard let vertex = library.makeFunction(name: "quadVertex"),
              let fragment = library.makeFunction(name: "quadFragment")
        else {
            throw SlabRuntimeError.invalidArgument("Slab Metal shader entry points are missing")
        }
        let descriptor = MTLRenderPipelineDescriptor()
        descriptor.label = "Slab painter"
        descriptor.vertexFunction = vertex
        descriptor.fragmentFunction = fragment
        descriptor.colorAttachments[0].pixelFormat = .bgra8Unorm_srgb
        descriptor.colorAttachments[0].isBlendingEnabled = true
        descriptor.colorAttachments[0].rgbBlendOperation = .add
        descriptor.colorAttachments[0].alphaBlendOperation = .add
        descriptor.colorAttachments[0].sourceRGBBlendFactor = .one
        descriptor.colorAttachments[0].sourceAlphaBlendFactor = .one
        descriptor.colorAttachments[0].destinationRGBBlendFactor = .oneMinusSourceAlpha
        descriptor.colorAttachments[0].destinationAlphaBlendFactor = .oneMinusSourceAlpha

        let samplerDescriptor = MTLSamplerDescriptor()
        samplerDescriptor.minFilter = .linear
        samplerDescriptor.magFilter = .linear
        samplerDescriptor.sAddressMode = .clampToEdge
        samplerDescriptor.tAddressMode = .clampToEdge
        guard let sampler = device.makeSamplerState(descriptor: samplerDescriptor) else {
            throw SlabRuntimeError.invalidArgument("Metal could not create a sampler")
        }

        self.device = device
        self.queue = queue
        pipeline = try device.makeRenderPipelineState(descriptor: descriptor)
        self.sampler = sampler
        textureLoader = MTKTextureLoader(device: device)
        dummyTexture = try Self.makeTexture(
            device: device,
            width: 1,
            height: 1,
            format: .rgba8Unorm,
            bytes: [255, 255, 255, 255],
            bytesPerRow: 4
        )
        super.init()

        view.device = device
        view.colorPixelFormat = .bgra8Unorm_srgb
        view.framebufferOnly = false
        view.autoResizeDrawable = true
        view.enableSetNeedsDisplay = false
        view.isPaused = true
        guard let layer = view.layer as? CAMetalLayer else {
            throw SlabRuntimeError.invalidArgument("MTKView did not create a CAMetalLayer")
        }
        layer.presentsWithTransaction = true
        view.delegate = self
    }

    func needs(_ reference: SlabGPUResourceRef, document: UInt32) -> Bool {
        let address = ResourceAddress(reference)
        if document == documentGeneration,
           resourceVersions[address] == reference.generation
        {
            return false
        }
        return stagedDocumentGeneration != document
            || stagedResourceVersions[address] != reference.generation
    }

    func install(_ value: SlabGPUResource, for reference: SlabGPUResourceRef, document: UInt32) throws {
        let resource: MetalResource
        switch value {
        case let .gradient(gradient):
            resource = .gradient(try makeGradient(gradient))
        case let .path(path):
            resource = .path(try makePath(path))
        case let .font(font):
            guard let provider = CGDataProvider(data: font.data as CFData),
                  let face = CGFont(provider)
            else {
                throw SlabRuntimeError.malformedResponse("Metal could not decode font \(font.family)")
            }
            resource = .font(face)
        case let .image(image):
            resource = .image(try makeImage(image))
        case let .shadow(shadow):
            resource = .shadow(shadow)
        }

        if stagedDocumentGeneration != document {
            stagedDocumentGeneration = document
            stagedResources.removeAll(keepingCapacity: true)
            stagedResourceVersions.removeAll(keepingCapacity: true)
        }
        let address = ResourceAddress(reference)
        stagedResources[address] = resource
        stagedResourceVersions[address] = reference.generation
    }

    func install(_ frame: SlabGPUFrame) throws {
        let decoded = try MetalFrame(frame)
        commitStagedResources(for: frame.documentGeneration)
        presentation = MetalPresentation(frame: frame, decoded: decoded)
    }


    func present(in view: MTKView) -> Bool {
        guard presentation != nil else { return true }
        explicitDraw = true
        drawSucceeded = false
        view.draw()
        explicitDraw = false
        return drawSucceeded
    }

    func draw(in view: MTKView) {
        guard explicitDraw, let presentation, let drawable = view.currentDrawable else { return }
        do {
            try encode(
                frame: presentation.frame,
                decoded: presentation.decoded,
                in: view,
                drawable: drawable
            )
        } catch {
            onError?(error)
        }
        drawSucceeded = true
    }

    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {
        sceneTexture = nil
        layerTextures.removeAll(keepingCapacity: true)
        auxiliaryTextures.removeAll(keepingCapacity: true)
    }

    private func commitStagedResources(for document: UInt32) {
        let changesDocument = document != documentGeneration
        if changesDocument {
            documentGeneration = document
            resources.removeAll(keepingCapacity: true)
            resourceVersions.removeAll(keepingCapacity: true)
            pathMasks.removeAll(keepingCapacity: true)
            textMasks.removeAll(keepingCapacity: true)
        }
        guard stagedDocumentGeneration == document else { return }

        var invalidatesPaths = false
        var invalidatesText = false
        for (address, resource) in stagedResources {
            resources[address] = resource
            invalidatesPaths = invalidatesPaths || address.kind == .path
            invalidatesText = invalidatesText || address.kind == .font
        }
        for (address, version) in stagedResourceVersions {
            resourceVersions[address] = version
        }
        stagedDocumentGeneration = nil
        stagedResources.removeAll(keepingCapacity: true)
        stagedResourceVersions.removeAll(keepingCapacity: true)
        if invalidatesPaths {
            pathMasks.removeAll(keepingCapacity: true)
        }
        if invalidatesText {
            textMasks.removeAll(keepingCapacity: true)
        }
    }

    private func encode(
        frame: SlabGPUFrame,
        decoded: MetalFrame,
        in view: MTKView,
        drawable: any CAMetalDrawable
    ) throws {
        let width = Int(view.drawableSize.width)
        let height = Int(view.drawableSize.height)
        guard width > 0, height > 0, view.bounds.width > 0 else { return }
        let scale = Float(view.drawableSize.width / view.bounds.width)
        let target = try sceneTarget(width: width, height: height)
        guard let commandBuffer = queue.makeCommandBuffer() else {
            throw SlabRuntimeError.invalidArgument("Metal could not create a command buffer")
        }
        commandBuffer.label = "Slab frame"
        let initial = PaintState(
            transform: matrix_identity_float3x3,
            clip: SIMD4(0, 0, Float(decoded.width), Float(decoded.height)),
            clipRadius: 0
        )
        let context = try RenderContext(
            renderer: self,
            commandBuffer: commandBuffer,
            target: target,
            clear: true,
            scale: scale
        )
        try render(
            frame: frame,
            decoded: decoded,
            range: decoded.operations.indices,
            context: context,
            state: initial,
            commandBuffer: commandBuffer,
            layerDepth: 0,
            scale: scale
        )
        context.end()

        let output = try RenderContext(
            renderer: self,
            commandBuffer: commandBuffer,
            target: drawable.texture,
            clear: true,
            scale: scale
        )
        var uniforms = DrawUniforms(
            rect: SIMD4(0, 0, Float(view.bounds.width), Float(view.bounds.height)),
            fill: SIMD4(1, 1, 1, 1),
            stroke: .zero,
            params: SIMD4(0, 0, 0, 1),
            uv: SIMD4(0, 0, 1, 1),
            clip: initial.clip,
            effect: SIMD4(0, -1, 0, DrawMode.composite.rawValue),
            extras: SIMD4(0, 0, 1, 1),
            paintBox: SIMD4(0, 0, Float(decoded.width), Float(decoded.height)),
            maskBox: .zero,
            maskParams: SIMD4(0, -1, 0, 1),
            transform: matrix_identity_float3x3,
            viewportScale: SIMD4(Float(width), Float(height), scale, 0)
        )
        encodeQuad(context: output, uniforms: &uniforms, texture: target)
        output.end()
        commandBuffer.present(drawable)
        commandBuffer.commit()
    }
    private func render(
        frame: SlabGPUFrame,
        decoded: MetalFrame,
        range: Range<Int>,
        context: RenderContext,
        state initialState: PaintState,
        commandBuffer: any MTLCommandBuffer,
        layerDepth: Int,
        scale: Float
    ) throws {
        var state = initialState
        var clips: [PaintState] = []
        var transforms: [simd_float3x3] = []
        var index = range.lowerBound
        while index < range.upperBound {
            let operation = decoded.operations[index]
            switch operation {
            case let .rect(rect):
                try draw(rect, state: state, context: context, scale: scale)
            case let .text(text):
                try draw(text, frame: frame, state: state, context: context, scale: scale)
            case let .image(image):
                draw(image, state: state, context: context)
            case let .path(path):
                try draw(path, frame: frame, state: state, context: context, scale: scale)
            case let .clipPush(clip):
                clips.append(state)
                state = clipped(state, by: clip)
            case .clipPop:
                if let previous = clips.popLast() {
                    state = previous
                }
            case let .rotatePush(rotation):
                transforms.append(state.transform)
                state.transform *= rotationMatrix(rotation)
            case .rotatePop:
                state.transform = transforms.popLast() ?? state.transform
            case let .scalePush(scaling):
                transforms.append(state.transform)
                state.transform *= scaleMatrix(scaling)
            case .scalePop:
                state.transform = transforms.popLast() ?? state.transform
            case let .tiltPush(tilt):
                transforms.append(state.transform)
                state.transform *= tiltMatrix(tilt)
            case .tiltPop:
                state.transform = transforms.popLast() ?? state.transform
            case let .groupPush(group):
                guard let end = matchingGroupEnd(in: decoded.operations, from: index, limit: range.upperBound) else {
                    throw SlabRuntimeError.malformedResponse("GPU frame has an unclosed group")
                }
                context.end()
                let layer = try layerTarget(depth: layerDepth, like: context.target)
                let child = try RenderContext(
                    renderer: self,
                    commandBuffer: commandBuffer,
                    target: layer,
                    clear: true,
                    scale: scale
                )
                try render(
                    frame: frame,
                    decoded: decoded,
                    range: (index + 1)..<end,
                    context: child,
                    state: state,
                    commandBuffer: commandBuffer,
                    layerDepth: layerDepth + 1,
                    scale: scale
                )
                child.end()
                let compositeTexture: any MTLTexture
                if group.blur > 0 {
                    let blurred = try auxiliaryTarget(index: 0, like: context.target)
                    let blur = MPSImageGaussianBlur(
                        device: device,
                        sigma: max(0.25, Float(group.blur) * scale / 2)
                    )
                    blur.encode(commandBuffer: commandBuffer, sourceTexture: layer, destinationTexture: blurred)
                    compositeTexture = blurred
                } else {
                    compositeTexture = layer
                }
                try context.resume()
                drawGroup(
                    group,
                    texture: compositeTexture,
                    state: state,
                    context: context
                )
                index = end
            case .groupPop:
                return
            case let .backdrop(backdrop):
                try drawBackdrop(
                    backdrop,
                    state: state,
                    context: context,
                    commandBuffer: commandBuffer,
                    scale: scale
                )
            }
            index += 1
        }
    }

    private func draw(
        _ rect: MetalRectOp,
        state: PaintState,
        context: RenderContext,
        scale: Float
    ) throws {
        if rect.shadowOffset >= 0, rect.shadowCount > 0 {
            let start = Int(rect.shadowOffset)
            let end = start + Int(rect.shadowCount)
            for index in start..<end {
                guard case let .shadow(shadow)? = resources[.init(kind: .shadow, index: UInt32(index))]
                else { continue }
                drawShadow(shadow, rect: rect, state: state, context: context)
            }
        }

        let background = paint(kind: rect.backgroundKind, value: rect.background, opacity: rect.opacity)
        let stroke = paint(kind: rect.strokeKind, value: rect.stroke, opacity: rect.opacity)
        guard background.visible || stroke.visible else { return }

        if rect.strokeSides != 15, stroke.visible, rect.strokeWidth > 0 {
            if background.visible {
                drawRectBody(rect, background: background, stroke: .none, state: state, context: context)
            }
            drawStrokeSides(rect, paint: stroke, state: state, context: context)
            return
        }
        drawRectBody(rect, background: background, stroke: stroke, state: state, context: context)
    }

    private func drawRectBody(
        _ rect: MetalRectOp,
        background: MetalPaint,
        stroke: MetalPaint,
        state: PaintState,
        context: RenderContext
    ) {
        let halfWidth = Float(max(0, rect.strokeWidth / 2))
        let strokeOffset: Float
        switch rect.strokeAlignment {
        case 1: strokeOffset = -halfWidth
        case 2: strokeOffset = halfWidth
        default: strokeOffset = 0
        }
        var uniforms = baseUniforms(
            rect: SIMD4(Float(rect.x), Float(rect.y), Float(rect.width), Float(rect.height)),
            radius: Float(rect.radius),
            state: state,
            mode: .rect
        )
        uniforms.fill = background.color
        uniforms.stroke = stroke.color
        uniforms.params.y = halfWidth
        uniforms.params.z = strokeOffset
        uniforms.effect.y = Float(background.gradientKind)
        uniforms.effect.z = background.gradientAngle
        uniforms.extras.x = Float(rect.smooth)
        uniforms.extras.z = Float(rect.grainAmount)
        uniforms.extras.w = Float(rect.grainSize)
        uniforms.paintBox = uniforms.rect
        encodeQuad(
            context: context,
            uniforms: &uniforms,
            gradient: background.gradientTexture,
            strokeGradient: stroke.gradientTexture
        )
    }

    private func drawStrokeSides(
        _ rect: MetalRectOp,
        paint: MetalPaint,
        state: PaintState,
        context: RenderContext
    ) {
        let width = Float(rect.strokeWidth)
        let shift: Float
        switch rect.strokeAlignment {
        case 1: shift = 0
        case 2: shift = -width
        default: shift = -width / 2
        }
        let x = Float(rect.x)
        let y = Float(rect.y)
        let w = Float(rect.width)
        let h = Float(rect.height)
        let bars: [(UInt32, SIMD4<Float>)] = [
            (1, SIMD4(x, y + shift, w, width)),
            (2, SIMD4(x + w - width - shift, y, width, h)),
            (4, SIMD4(x, y + h - width - shift, w, width)),
            (8, SIMD4(x + shift, y, width, h)),
        ]
        for (side, bar) in bars where rect.strokeSides & side != 0 {
            var uniforms = baseUniforms(rect: bar, radius: 0, state: state, mode: .rect)
            uniforms.fill = paint.color
            uniforms.effect.y = Float(paint.gradientKind)
            uniforms.effect.z = paint.gradientAngle
            uniforms.paintBox = SIMD4(x, y, w, h)
            encodeQuad(context: context, uniforms: &uniforms, gradient: paint.gradientTexture)
        }
    }

    private func drawShadow(
        _ shadow: SlabGPUShadow,
        rect: MetalRectOp,
        state: PaintState,
        context: RenderContext
    ) {
        let spread = shadow.inset ? 0 : shadow.spread
        var uniforms = baseUniforms(
            rect: SIMD4(
                Float(rect.x + shadow.x - spread),
                Float(rect.y + shadow.y - spread),
                Float(rect.width + 2 * spread),
                Float(rect.height + 2 * spread)
            ),
            radius: Float(max(0, rect.radius + spread)),
            state: state,
            mode: shadow.inset ? .insetShadow : .shadow
        )
        uniforms.fill = packedColor(shadow.color, opacity: rect.opacity)
        uniforms.extras.y = Float(max(0.5, shadow.blur / 2))
        if shadow.inset {
            uniforms.rect = SIMD4(Float(rect.x), Float(rect.y), Float(rect.width), Float(rect.height))
            uniforms.extras.z = Float(shadow.x)
            uniforms.extras.w = Float(shadow.y)
        }
        encodeQuad(context: context, uniforms: &uniforms)
    }

    private func draw(
        _ text: MetalTextOp,
        frame: SlabGPUFrame,
        state: PaintState,
        context: RenderContext,
        scale: Float
    ) throws {
        guard text.string >= 0,
              Int(text.string) < frame.strings.count,
              !frame.strings[Int(text.string)].isEmpty,
              text.size > 0
        else { return }
        let value = frame.strings[Int(text.string)]
        let mask = try textMask(
            text: value,
            fontIndex: text.font,
            size: text.size,
            tracking: text.tracking,
            measuredWidth: text.measuredWidth,
            scale: scale
        )
        let paint = paint(kind: text.colorKind, value: text.color, opacity: text.opacity)
        var uniforms = baseUniforms(
            rect: SIMD4(
                Float(text.x) + mask.originX,
                Float(text.baseline) - mask.baseline,
                mask.width,
                mask.height
            ),
            radius: 0,
            state: state,
            mode: .mask
        )
        uniforms.fill = paint.color
        uniforms.effect.y = Float(paint.gradientKind)
        uniforms.effect.z = paint.gradientAngle
        uniforms.paintBox = SIMD4(
            Float(text.gradientBox.x),
            Float(text.gradientBox.y),
            Float(text.gradientBox.z),
            Float(text.gradientBox.w)
        )
        encodeQuad(
            context: context,
            uniforms: &uniforms,
            texture: mask.texture,
            gradient: paint.gradientTexture
        )
        if text.strike, text.measuredWidth > 0 {
            let thickness = max(1 / Double(scale), text.size / 14)
            var strike = baseUniforms(
                rect: SIMD4(
                    Float(text.x),
                    Float(text.baseline - text.size * 0.32),
                    Float(text.measuredWidth),
                    Float(thickness)
                ),
                radius: 0,
                state: state,
                mode: .rect
            )
            strike.fill = paint.color
            strike.effect.y = Float(paint.gradientKind)
            strike.effect.z = paint.gradientAngle
            strike.paintBox = uniforms.paintBox
            encodeQuad(context: context, uniforms: &strike, gradient: paint.gradientTexture)
        }
    }

    private func draw(_ image: MetalImageOp, state: PaintState, context: RenderContext) {
        guard image.image >= 0,
              case let .image(texture)? = resources[.init(kind: .image, index: UInt32(image.image))]
        else { return }
        let imageWidth = Double(texture.width)
        let imageHeight = Double(texture.height)
        guard imageWidth > 0, imageHeight > 0, image.width > 0, image.height > 0 else { return }
        let scaleX: Double
        let scaleY: Double
        switch image.fit {
        case 1:
            let scale = min(image.width / imageWidth, image.height / imageHeight)
            scaleX = scale
            scaleY = scale
        case 2:
            scaleX = image.width / imageWidth
            scaleY = image.height / imageHeight
        default:
            let scale = max(image.width / imageWidth, image.height / imageHeight)
            scaleX = scale
            scaleY = scale
        }
        let translateX = (image.width - imageWidth * scaleX) / 2
        let translateY = (image.height - imageHeight * scaleY) / 2
        let u0 = -translateX / (imageWidth * scaleX)
        let v0 = -translateY / (imageHeight * scaleY)
        var uniforms = baseUniforms(
            rect: SIMD4(Float(image.x), Float(image.y), Float(image.width), Float(image.height)),
            radius: Float(image.radius),
            state: state,
            mode: .image
        )
        uniforms.params.w = Float(image.opacity)
        uniforms.uv = SIMD4(
            Float(u0),
            Float(v0),
            Float(image.width / (imageWidth * scaleX)),
            Float(image.height / (imageHeight * scaleY))
        )
        uniforms.extras.x = image.fit == 1 ? 1 : 0
        encodeQuad(context: context, uniforms: &uniforms, texture: texture)
    }

    private func draw(
        _ path: MetalPathOp,
        frame: SlabGPUFrame,
        state: PaintState,
        context: RenderContext,
        scale: Float
    ) throws {
        let source: CGPath
        let identity: Int
        if path.path >= 0 {
            guard case let .path(retained)? = resources[.init(kind: .path, index: UInt32(path.path))]
            else { return }
            source = retained
            identity = Int(path.path)
        } else {
            let runtimeIndex = Int(~path.path)
            guard runtimeIndex >= 0, runtimeIndex < frame.runtimePaths.count else { return }
            source = try makePath(frame.runtimePaths[runtimeIndex])
            var hasher = Hasher()
            hasher.combine(frame.runtimePaths[runtimeIndex].verbs)
            hasher.combine(frame.runtimePaths[runtimeIndex].coordinates)
            identity = hasher.finalize()
        }
        if path.backgroundKind != 0 {
            let mask = try pathMask(
                path: source,
                identity: identity,
                strokeWidth: 0,
                dashOn: 0,
                dashOff: 0,
                scale: scale
            )
            drawPathMask(mask, operation: path, paint: paint(
                kind: path.backgroundKind,
                value: path.background,
                opacity: path.opacity
            ), state: state, context: context)
        }
        if path.strokeKind != 0, path.strokeWidth > 0 {
            let mask = try pathMask(
                path: source,
                identity: identity,
                strokeWidth: path.strokeWidth,
                dashOn: path.dashed ? path.dashOn : 0,
                dashOff: path.dashed ? path.dashOff : 0,
                scale: scale
            )
            drawPathMask(mask, operation: path, paint: paint(
                kind: path.strokeKind,
                value: path.stroke,
                opacity: path.opacity
            ), state: state, context: context)
        }
    }

    private func drawPathMask(
        _ mask: PathMask,
        operation: MetalPathOp,
        paint: MetalPaint,
        state: PaintState,
        context: RenderContext
    ) {
        var uniforms = baseUniforms(
            rect: SIMD4(
                mask.x + Float(operation.x),
                mask.y + Float(operation.y),
                mask.width,
                mask.height
            ),
            radius: 0,
            state: state,
            mode: .mask
        )
        uniforms.fill = paint.color
        uniforms.effect.y = Float(paint.gradientKind)
        uniforms.effect.z = paint.gradientAngle
        uniforms.paintBox = SIMD4(
            mask.pathX + Float(operation.x),
            mask.pathY + Float(operation.y),
            mask.pathWidth,
            mask.pathHeight
        )
        encodeQuad(
            context: context,
            uniforms: &uniforms,
            texture: mask.texture,
            gradient: paint.gradientTexture
        )
    }

    private func drawGroup(
        _ group: MetalGroupOp,
        texture: any MTLTexture,
        state: PaintState,
        context: RenderContext
    ) {
        let width = Float(context.target.width) / context.scale
        let height = Float(context.target.height) / context.scale
        var uniforms = baseUniforms(
            rect: SIMD4(0, 0, width, height),
            radius: 0,
            state: state,
            mode: .composite
        )
        uniforms.params.w = Float(group.opacity)
        uniforms.uv = SIMD4(0, 0, 1, 1)
        configureMask(
            kind: group.maskKind,
            value: group.mask,
            box: group.maskBox,
            uniforms: &uniforms
        )
        encodeQuad(
            context: context,
            uniforms: &uniforms,
            texture: texture,
            mask: maskTexture(kind: group.maskKind, value: group.mask)
        )
    }

    private func drawBackdrop(
        _ backdrop: MetalBackdropOp,
        state: PaintState,
        context: RenderContext,
        commandBuffer: any MTLCommandBuffer,
        scale: Float
    ) throws {
        context.end()
        let copy = try auxiliaryTarget(index: 0, like: context.target)
        guard let blit = commandBuffer.makeBlitCommandEncoder() else {
            throw SlabRuntimeError.invalidArgument("Metal could not create a backdrop blit encoder")
        }
        blit.copy(
            from: context.target,
            sourceSlice: 0,
            sourceLevel: 0,
            sourceOrigin: .init(x: 0, y: 0, z: 0),
            sourceSize: .init(width: context.target.width, height: context.target.height, depth: 1),
            to: copy,
            destinationSlice: 0,
            destinationLevel: 0,
            destinationOrigin: .init(x: 0, y: 0, z: 0)
        )
        blit.endEncoding()
        let source: any MTLTexture
        if backdrop.blur > 0 {
            let blurred = try auxiliaryTarget(index: 1, like: context.target)
            let blur = MPSImageGaussianBlur(
                device: device,
                sigma: max(0.25, Float(backdrop.blur) * scale / 2)
            )
            blur.encode(commandBuffer: commandBuffer, sourceTexture: copy, destinationTexture: blurred)
            source = blurred
        } else {
            source = copy
        }
        try context.resume()
        var uniforms = baseUniforms(
            rect: SIMD4(
                Float(backdrop.rect.x),
                Float(backdrop.rect.y),
                Float(backdrop.rect.z),
                Float(backdrop.rect.w)
            ),
            radius: Float(backdrop.radius),
            state: state,
            mode: .composite
        )
        let targetWidth = Double(context.target.width) / Double(context.scale)
        let targetHeight = Double(context.target.height) / Double(context.scale)
        uniforms.uv = SIMD4(
            Float(backdrop.rect.x / targetWidth),
            Float(backdrop.rect.y / targetHeight),
            Float(backdrop.rect.z / targetWidth),
            Float(backdrop.rect.w / targetHeight)
        )
        uniforms.extras.z = Float(backdrop.saturation)
        uniforms.extras.w = Float(backdrop.brightness)
        configureMask(
            kind: backdrop.maskKind,
            value: backdrop.mask,
            box: backdrop.rect,
            uniforms: &uniforms
        )
        encodeQuad(
            context: context,
            uniforms: &uniforms,
            texture: source,
            mask: maskTexture(kind: backdrop.maskKind, value: backdrop.mask)
        )
    }

    private func configureMask(
        kind: UInt32,
        value: UInt32,
        box: SIMD4<Double>,
        uniforms: inout DrawUniforms
    ) {
        guard kind != 0 else { return }
        uniforms.maskBox = SIMD4(Float(box.x), Float(box.y), Float(box.z), Float(box.w))
        if kind == 1 {
            uniforms.maskParams = SIMD4(1, -1, 0, packedColor(value, opacity: 1).w)
        } else if case let .gradient(gradient)? = resources[.init(kind: .gradient, index: value)] {
            uniforms.maskParams = SIMD4(1, Float(gradient.kind), gradient.angle, 1)
        }
    }

    private func maskTexture(kind: UInt32, value: UInt32) -> (any MTLTexture)? {
        guard kind == 2,
              case let .gradient(gradient)? = resources[.init(kind: .gradient, index: value)]
        else { return nil }
        return gradient.texture
    }

    private func baseUniforms(
        rect: SIMD4<Float>,
        radius: Float,
        state: PaintState,
        mode: DrawMode
    ) -> DrawUniforms {
        DrawUniforms(
            rect: rect,
            fill: .zero,
            stroke: .zero,
            params: SIMD4(radius, 0, 0, 1),
            uv: SIMD4(0, 0, 1, 1),
            clip: state.clip,
            effect: SIMD4(state.clipRadius, -1, 0, mode.rawValue),
            extras: SIMD4(0, 0, 1, 1),
            paintBox: rect,
            maskBox: .zero,
            maskParams: SIMD4(0, -1, 0, 1),
            transform: state.transform,
            viewportScale: .zero
        )
    }

    private func encodeQuad(
        context: RenderContext,
        uniforms: inout DrawUniforms,
        texture: (any MTLTexture)? = nil,
        gradient: (any MTLTexture)? = nil,
        strokeGradient: (any MTLTexture)? = nil,
        mask: (any MTLTexture)? = nil
    ) {
        uniforms.viewportScale = SIMD4(
            Float(context.target.width),
            Float(context.target.height),
            context.scale,
            0
        )
        let encoder = context.encoder
        encoder.setRenderPipelineState(pipeline)
        encoder.setVertexBytes(&uniforms, length: MemoryLayout<DrawUniforms>.stride, index: 0)
        encoder.setFragmentBytes(&uniforms, length: MemoryLayout<DrawUniforms>.stride, index: 0)
        encoder.setFragmentTexture(texture ?? dummyTexture, index: 0)
        encoder.setFragmentTexture(gradient ?? dummyTexture, index: 1)
        encoder.setFragmentTexture(strokeGradient ?? dummyTexture, index: 2)
        encoder.setFragmentTexture(mask ?? dummyTexture, index: 3)
        encoder.setFragmentSamplerState(sampler, index: 0)
        encoder.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4)
    }

    private func paint(kind: UInt32, value: UInt32, opacity: Double) -> MetalPaint {
        switch kind {
        case 1:
            return MetalPaint(color: packedColor(value, opacity: opacity))
        case 2:
            guard case let .gradient(gradient)? = resources[.init(kind: .gradient, index: value)]
            else { return .none }
            return MetalPaint(
                color: SIMD4(0, 0, 0, Float(opacity)),
                gradientTexture: gradient.texture,
                gradientKind: Int32(gradient.kind),
                gradientAngle: gradient.angle
            )
        default:
            return .none
        }
    }

    private func packedColor(_ value: UInt32, opacity: Double) -> SIMD4<Float> {
        SIMD4(
            Float(value & 0xff) / 255,
            Float((value >> 8) & 0xff) / 255,
            Float((value >> 16) & 0xff) / 255,
            Float((value >> 24) & 0xff) / 255 * Float(opacity)
        )
    }

    private func makeGradient(_ gradient: SlabGPUGradient) throws -> GradientResource {
        let width = 256
        var bytes = [UInt8](repeating: 0, count: width * 4)
        for sample in 0..<width {
            let position = Double(sample) / Double(width - 1)
            let color = gradientColor(at: position, stops: gradient.stops)
            bytes[sample * 4] = UInt8(color & 0xff)
            bytes[sample * 4 + 1] = UInt8((color >> 8) & 0xff)
            bytes[sample * 4 + 2] = UInt8((color >> 16) & 0xff)
            bytes[sample * 4 + 3] = UInt8((color >> 24) & 0xff)
        }
        return GradientResource(
            texture: try Self.makeTexture(
                device: device,
                width: width,
                height: 1,
                format: .rgba8Unorm_srgb,
                bytes: bytes,
                bytesPerRow: width * 4
            ),
            kind: gradient.kind,
            angle: Float(gradient.angle)
        )
    }

    private func gradientColor(at position: Double, stops: [SlabGPUGradientStop]) -> UInt32 {
        guard let first = stops.first else { return 0 }
        if position <= first.position { return first.color }
        var previous = first
        for stop in stops.dropFirst() {
            if position <= stop.position {
                let denominator = max(0.000_001, stop.position - previous.position)
                let amount = min(1, max(0, (position - previous.position) / denominator))
                var result: UInt32 = 0
                for component in 0..<4 {
                    let shift = UInt32(component * 8)
                    let lhs = Double((previous.color >> shift) & 0xff)
                    let rhs = Double((stop.color >> shift) & 0xff)
                    let value = UInt32((lhs + (rhs - lhs) * amount).rounded())
                    result |= value << shift
                }
                return result
            }
            previous = stop
        }
        return previous.color
    }

    private func makeImage(_ image: SlabGPUImage) throws -> any MTLTexture {
        if image.format == 0 {
            guard let source = CGImageSourceCreateWithData(image.data as CFData, nil),
                  let cgImage = CGImageSourceCreateImageAtIndex(source, 0, nil)
            else {
                throw SlabRuntimeError.malformedResponse("Metal could not decode PNG image")
            }
            return try textureLoader.newTexture(
                cgImage: cgImage,
                options: [.SRGB: true, .textureUsage: MTLTextureUsage.shaderRead.rawValue]
            )
        }
        guard image.format == 1,
              let width = Int(exactly: image.width),
              let height = Int(exactly: image.height),
              image.data.count == width * height * 4
        else {
            throw SlabRuntimeError.malformedResponse("Metal image payload has invalid dimensions")
        }
        var bytes = [UInt8](image.data)
        for offset in stride(from: 0, to: bytes.count, by: 4) {
            let alpha = UInt16(bytes[offset + 3])
            bytes[offset] = UInt8(UInt16(bytes[offset]) * alpha / 255)
            bytes[offset + 1] = UInt8(UInt16(bytes[offset + 1]) * alpha / 255)
            bytes[offset + 2] = UInt8(UInt16(bytes[offset + 2]) * alpha / 255)
        }
        return try Self.makeTexture(
            device: device,
            width: width,
            height: height,
            format: .rgba8Unorm_srgb,
            bytes: bytes,
            bytesPerRow: width * 4
        )
    }

    private func makePath(_ path: SlabGPUPath) throws -> CGPath {
        let result = CGMutablePath()
        var coordinate = 0
        for verb in path.verbs {
            func take(_ count: Int) throws -> ArraySlice<Double> {
                guard coordinate + count <= path.coordinates.count else {
                    throw SlabRuntimeError.malformedResponse("GPU path coordinate stream is truncated")
                }
                defer { coordinate += count }
                return path.coordinates[coordinate..<(coordinate + count)]
            }
            switch verb {
            case 0:
                let values = try take(2)
                result.move(to: CGPoint(x: values[values.startIndex], y: values[values.startIndex + 1]))
            case 1:
                let values = try take(2)
                result.addLine(to: CGPoint(x: values[values.startIndex], y: values[values.startIndex + 1]))
            case 2:
                let values = try take(6)
                result.addCurve(
                    to: CGPoint(x: values[values.startIndex + 4], y: values[values.startIndex + 5]),
                    control1: CGPoint(x: values[values.startIndex], y: values[values.startIndex + 1]),
                    control2: CGPoint(x: values[values.startIndex + 2], y: values[values.startIndex + 3])
                )
            case 3:
                let values = try take(4)
                result.addQuadCurve(
                    to: CGPoint(x: values[values.startIndex + 2], y: values[values.startIndex + 3]),
                    control: CGPoint(x: values[values.startIndex], y: values[values.startIndex + 1])
                )
            case 4:
                result.closeSubpath()
            default:
                throw SlabRuntimeError.malformedResponse("GPU path has unknown verb \(verb)")
            }
        }
        guard coordinate == path.coordinates.count else {
            throw SlabRuntimeError.malformedResponse("GPU path has trailing coordinates")
        }
        return result
    }

    private func pathMask(
        path: CGPath,
        identity: Int,
        strokeWidth: Double,
        dashOn: Double,
        dashOff: Double,
        scale: Float
    ) throws -> PathMask {
        let key = PathMaskKey(
            identity: identity,
            strokeWidth: Int((strokeWidth * Double(scale) * 100).rounded()),
            dashOn: Int((dashOn * Double(scale) * 100).rounded()),
            dashOff: Int((dashOff * Double(scale) * 100).rounded()),
            scale: Int((scale * 100).rounded())
        )
        if let cached = pathMasks[key] { return cached }
        let pathBounds = path.boundingBoxOfPath
        let padding = max(2, CGFloat(strokeWidth * Double(scale) / 2 + 2))
        let width = max(1, Int(ceil(pathBounds.width * CGFloat(scale) + padding * 2)))
        let height = max(1, Int(ceil(pathBounds.height * CGFloat(scale) + padding * 2)))
        var pixels = [UInt8](repeating: 0, count: width * height)
        try pixels.withUnsafeMutableBytes { storage in
            guard let context = CGContext(
                data: storage.baseAddress,
                width: width,
                height: height,
                bitsPerComponent: 8,
                bytesPerRow: width,
                space: CGColorSpaceCreateDeviceGray(),
                bitmapInfo: CGImageAlphaInfo.none.rawValue
            ) else {
                throw SlabRuntimeError.invalidArgument("Core Graphics could not rasterize a Slab path")
            }
            context.translateBy(x: 0, y: CGFloat(height))
            context.scaleBy(x: 1, y: -1)
            context.scaleBy(x: CGFloat(scale), y: CGFloat(scale))
            context.translateBy(
                x: -pathBounds.minX + padding / CGFloat(scale),
                y: -pathBounds.minY + padding / CGFloat(scale)
            )
            context.addPath(path)
            context.setShouldAntialias(true)
            if strokeWidth > 0 {
                context.setStrokeColor(gray: 1, alpha: 1)
                context.setLineWidth(CGFloat(strokeWidth))
                context.setLineCap(CGLineCap.butt)
                context.setLineJoin(CGLineJoin.miter)
                if dashOn > 0, dashOff > 0 {
                    context.setLineDash(phase: 0, lengths: [CGFloat(dashOn), CGFloat(dashOff)])
                }
                context.strokePath()
            } else {
                context.setFillColor(gray: 1, alpha: 1)
                context.fillPath(using: CGPathFillRule.winding)
            }
        }
        let texture = try Self.makeTexture(
            device: device,
            width: width,
            height: height,
            format: .r8Unorm,
            bytes: pixels,
            bytesPerRow: width
        )
        let result = PathMask(
            texture: texture,
            x: Float(pathBounds.minX - padding / CGFloat(scale)),
            y: Float(pathBounds.minY - padding / CGFloat(scale)),
            width: Float(width) / scale,
            height: Float(height) / scale,
            pathX: Float(pathBounds.minX),
            pathY: Float(pathBounds.minY),
            pathWidth: Float(pathBounds.width),
            pathHeight: Float(pathBounds.height)
        )
        pathMasks[key] = result
        return result
    }

    private func textMask(
        text: String,
        fontIndex: Int32,
        size: Double,
        tracking: Double,
        measuredWidth: Double,
        scale: Float
    ) throws -> TextMask {
        let key = TextMaskKey(
            text: text,
            font: fontIndex,
            size: Int((size * Double(scale) * 100).rounded()),
            tracking: Int((tracking * Double(scale) * 100).rounded()),
            width: Int((measuredWidth * Double(scale) * 100).rounded())
        )
        if let cached = textMasks[key] { return cached }
        let pointSize = CGFloat(size * Double(scale))
        let font: CTFont
        if fontIndex >= 0,
           case let .font(face)? = resources[.init(kind: .font, index: UInt32(fontIndex))]
        {
            font = CTFontCreateWithGraphicsFont(face, pointSize, nil, nil)
        } else {
            font = CTFontCreateWithName("system-ui" as CFString, pointSize, nil)
        }
        let attributes: [NSAttributedString.Key: Any] = [
            NSAttributedString.Key(kCTFontAttributeName as String): font,
            NSAttributedString.Key(kCTKernAttributeName as String): CGFloat(tracking * Double(scale)),
            NSAttributedString.Key(kCTForegroundColorFromContextAttributeName as String): true,
        ]
        let line = CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: attributes))
        let ascent = CTFontGetAscent(font)
        let descent = CTFontGetDescent(font)
        let leading = CTFontGetLeading(font)
        let padding: CGFloat = 2
        let width = max(1, Int(ceil(max(CGFloat(measuredWidth) * CGFloat(scale), CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))) + padding * 2)))
        let height = max(1, Int(ceil(ascent + descent + leading + padding * 2)))
        var pixels = [UInt8](repeating: 0, count: width * height)
        try pixels.withUnsafeMutableBytes { storage in
            guard let context = CGContext(
                data: storage.baseAddress,
                width: width,
                height: height,
                bitsPerComponent: 8,
                bytesPerRow: width,
                space: CGColorSpaceCreateDeviceGray(),
                bitmapInfo: CGImageAlphaInfo.none.rawValue
            ) else {
                throw SlabRuntimeError.invalidArgument("Core Graphics could not rasterize Slab text")
            }
            context.setShouldAntialias(true)
            context.setFillColor(gray: 1, alpha: 1)
            context.setTextDrawingMode(CGTextDrawingMode.fill)
            context.textMatrix = CGAffineTransform.identity
            context.textPosition = CGPoint(x: padding, y: descent + padding)
            CTLineDraw(line, context)
        }
        let texture = try Self.makeTexture(
            device: device,
            width: width,
            height: height,
            format: .r8Unorm,
            bytes: pixels,
            bytesPerRow: width
        )
        let result = TextMask(
            texture: texture,
            originX: -Float(padding / CGFloat(scale)),
            baseline: Float((ascent + padding) / CGFloat(scale)),
            width: Float(width) / scale,
            height: Float(height) / scale
        )
        textMasks[key] = result
        return result
    }

    private func sceneTarget(width: Int, height: Int) throws -> any MTLTexture {
        if let sceneTexture, sceneTexture.width == width, sceneTexture.height == height {
            return sceneTexture
        }
        layerTextures.removeAll(keepingCapacity: true)
        auxiliaryTextures.removeAll(keepingCapacity: true)
        let texture = try renderTexture(width: width, height: height)
        sceneTexture = texture
        return texture
    }

    private func layerTarget(depth: Int, like target: any MTLTexture) throws -> any MTLTexture {
        while layerTextures.count <= depth {
            layerTextures.append(try renderTexture(width: target.width, height: target.height))
        }
        return layerTextures[depth]
    }

    private func auxiliaryTarget(index: Int, like target: any MTLTexture) throws -> any MTLTexture {
        while auxiliaryTextures.count <= index {
            auxiliaryTextures.append(try renderTexture(width: target.width, height: target.height))
        }
        return auxiliaryTextures[index]
    }

    private func renderTexture(width: Int, height: Int) throws -> any MTLTexture {
        let descriptor = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: .bgra8Unorm_srgb,
            width: width,
            height: height,
            mipmapped: false
        )
        descriptor.usage = [.renderTarget, .shaderRead, .shaderWrite]
        descriptor.storageMode = .private
        guard let texture = device.makeTexture(descriptor: descriptor) else {
            throw SlabRuntimeError.invalidArgument("Metal could not allocate a render texture")
        }
        return texture
    }

    private static func makeTexture(
        device: any MTLDevice,
        width: Int,
        height: Int,
        format: MTLPixelFormat,
        bytes: [UInt8],
        bytesPerRow: Int
    ) throws -> any MTLTexture {
        let descriptor = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: format,
            width: width,
            height: height,
            mipmapped: false
        )
        descriptor.usage = .shaderRead
        guard let texture = device.makeTexture(descriptor: descriptor) else {
            throw SlabRuntimeError.invalidArgument("Metal could not allocate a resource texture")
        }
        bytes.withUnsafeBytes { source in
            texture.replace(
                region: MTLRegionMake2D(0, 0, width, height),
                mipmapLevel: 0,
                withBytes: source.baseAddress!,
                bytesPerRow: bytesPerRow
            )
        }
        return texture
    }

    private func clipped(_ state: PaintState, by clip: MetalClipOp) -> PaintState {
        let points = [
            transformPoint(SIMD2(Float(clip.x), Float(clip.y)), by: state.transform),
            transformPoint(SIMD2(Float(clip.x + clip.width), Float(clip.y)), by: state.transform),
            transformPoint(SIMD2(Float(clip.x), Float(clip.y + clip.height)), by: state.transform),
            transformPoint(SIMD2(Float(clip.x + clip.width), Float(clip.y + clip.height)), by: state.transform),
        ]
        let x0 = points.map(\.x).min() ?? 0
        let y0 = points.map(\.y).min() ?? 0
        let x1 = points.map(\.x).max() ?? 0
        let y1 = points.map(\.y).max() ?? 0
        return PaintState(
            transform: state.transform,
            clip: SIMD4(
                max(state.clip.x, x0),
                max(state.clip.y, y0),
                min(state.clip.z, x1),
                min(state.clip.w, y1)
            ),
            clipRadius: isAffineIdentity(state.transform) ? Float(clip.radius) : 0
        )
    }

    private func matchingGroupEnd(in operations: [MetalFrameOp], from start: Int, limit: Int) -> Int? {
        var depth = 0
        for index in (start + 1)..<limit {
            switch operations[index] {
            case .groupPush: depth += 1
            case .groupPop where depth == 0: return index
            case .groupPop: depth -= 1
            default: break
            }
        }
        return nil
    }
}

private final class RenderContext {
    let target: any MTLTexture
    let scale: Float
    private unowned let renderer: MetalRenderer
    private let commandBuffer: any MTLCommandBuffer
    private(set) var encoder: any MTLRenderCommandEncoder
    private var active = true

    init(
        renderer: MetalRenderer,
        commandBuffer: any MTLCommandBuffer,
        target: any MTLTexture,
        clear: Bool,
        scale: Float
    ) throws {
        self.renderer = renderer
        self.commandBuffer = commandBuffer
        self.target = target
        self.scale = scale
        guard let encoder = Self.makeEncoder(commandBuffer: commandBuffer, target: target, clear: clear)
        else {
            throw SlabRuntimeError.invalidArgument("Metal could not create a render encoder")
        }
        self.encoder = encoder
    }

    func end() {
        guard active else { return }
        encoder.endEncoding()
        active = false
    }

    func resume() throws {
        guard !active else { return }
        guard let encoder = Self.makeEncoder(commandBuffer: commandBuffer, target: target, clear: false)
        else {
            throw SlabRuntimeError.invalidArgument("Metal could not resume a render encoder")
        }
        self.encoder = encoder
        active = true
    }

    private static func makeEncoder(
        commandBuffer: any MTLCommandBuffer,
        target: any MTLTexture,
        clear: Bool
    ) -> (any MTLRenderCommandEncoder)? {
        let descriptor = MTLRenderPassDescriptor()
        descriptor.colorAttachments[0].texture = target
        descriptor.colorAttachments[0].loadAction = clear ? .clear : .load
        descriptor.colorAttachments[0].storeAction = .store
        descriptor.colorAttachments[0].clearColor = MTLClearColorMake(0, 0, 0, 0)
        return commandBuffer.makeRenderCommandEncoder(descriptor: descriptor)
    }
}

private struct MetalPresentation {
    let frame: SlabGPUFrame
    let decoded: MetalFrame
}

private struct ResourceAddress: Hashable {
    let kind: SlabGPUResourceKind
    let index: UInt32

    init(kind: SlabGPUResourceKind, index: UInt32) {
        self.kind = kind
        self.index = index
    }

    init(_ reference: SlabGPUResourceRef) {
        kind = reference.kind
        index = reference.index
    }
}

private enum MetalResource {
    case gradient(GradientResource)
    case path(CGPath)
    case font(CGFont)
    case image(any MTLTexture)
    case shadow(SlabGPUShadow)
}

private struct GradientResource {
    let texture: any MTLTexture
    let kind: UInt32
    let angle: Float
}

private struct PathMask {
    let texture: any MTLTexture
    let x: Float
    let y: Float
    let width: Float
    let height: Float
    let pathX: Float
    let pathY: Float
    let pathWidth: Float
    let pathHeight: Float
}

private struct PathMaskKey: Hashable {
    let identity: Int
    let strokeWidth: Int
    let dashOn: Int
    let dashOff: Int
    let scale: Int
}

private struct TextMask {
    let texture: any MTLTexture
    let originX: Float
    let baseline: Float
    let width: Float
    let height: Float
}

private struct TextMaskKey: Hashable {
    let text: String
    let font: Int32
    let size: Int
    let tracking: Int
    let width: Int
}

private struct MetalPaint {
    let color: SIMD4<Float>
    let gradientTexture: (any MTLTexture)?
    let gradientKind: Int32
    let gradientAngle: Float

    init(
        color: SIMD4<Float>,
        gradientTexture: (any MTLTexture)? = nil,
        gradientKind: Int32 = -1,
        gradientAngle: Float = 0
    ) {
        self.color = color
        self.gradientTexture = gradientTexture
        self.gradientKind = gradientKind
        self.gradientAngle = gradientAngle
    }

    var visible: Bool { color.w > 0 || gradientTexture != nil }
    static var none: MetalPaint { MetalPaint(color: .zero) }
}

private struct PaintState {
    var transform: simd_float3x3
    var clip: SIMD4<Float>
    var clipRadius: Float
}

private enum DrawMode: Float {
    case rect = 0
    case image = 1
    case mask = 2
    case composite = 3
    case shadow = 4
    case insetShadow = 5
}

private struct DrawUniforms {
    var rect: SIMD4<Float>
    var fill: SIMD4<Float>
    var stroke: SIMD4<Float>
    var params: SIMD4<Float>
    var uv: SIMD4<Float>
    var clip: SIMD4<Float>
    var effect: SIMD4<Float>
    var extras: SIMD4<Float>
    var paintBox: SIMD4<Float>
    var maskBox: SIMD4<Float>
    var maskParams: SIMD4<Float>
    var transform: simd_float3x3
    var viewportScale: SIMD4<Float>
}

private func rotationMatrix(_ rotation: MetalRotateOp) -> simd_float3x3 {
    let radians = Float(rotation.degrees * .pi / 180)
    let cosine = cos(radians)
    let sine = sin(radians)
    let matrix = simd_float3x3(columns: (
        SIMD3(cosine, sine, 0),
        SIMD3(-sine, cosine, 0),
        SIMD3(0, 0, 1)
    ))
    return translation(Float(rotation.center.x), Float(rotation.center.y))
        * matrix
        * translation(-Float(rotation.center.x), -Float(rotation.center.y))
}

private func scaleMatrix(_ scale: MetalScaleOp) -> simd_float3x3 {
    let matrix = simd_float3x3(diagonal: SIMD3(Float(scale.scale.x), Float(scale.scale.y), 1))
    return translation(Float(scale.center.x), Float(scale.center.y))
        * matrix
        * translation(-Float(scale.center.x), -Float(scale.center.y))
}

private func tiltMatrix(_ tilt: MetalTiltOp) -> simd_float3x3 {
    let x = Float(tilt.xDegrees * .pi / 180)
    let y = Float(tilt.yDegrees * .pi / 180)
    let depth = max(1, Float(tilt.depth))
    let matrix = simd_float3x3(columns: (
        SIMD3(cos(y), 0, sin(y) / depth),
        SIMD3(sin(y) * sin(x), cos(x), -cos(y) * sin(x) / depth),
        SIMD3(0, 0, 1)
    ))
    return translation(Float(tilt.center.x), Float(tilt.center.y))
        * matrix
        * translation(-Float(tilt.center.x), -Float(tilt.center.y))
}

private func translation(_ x: Float, _ y: Float) -> simd_float3x3 {
    simd_float3x3(columns: (
        SIMD3(1, 0, 0),
        SIMD3(0, 1, 0),
        SIMD3(x, y, 1)
    ))
}

private func transformPoint(_ point: SIMD2<Float>, by matrix: simd_float3x3) -> SIMD2<Float> {
    let transformed = matrix * SIMD3(point.x, point.y, 1)
    let denominator = max(0.000_001, transformed.z)
    return SIMD2(transformed.x / denominator, transformed.y / denominator)
}

private func isAffineIdentity(_ matrix: simd_float3x3) -> Bool {
    matrix.columns.0 == SIMD3<Float>(1, 0, 0)
        && matrix.columns.1 == SIMD3<Float>(0, 1, 0)
        && matrix.columns.2 == SIMD3<Float>(0, 0, 1)
}

private let metalShader = #"""
#include <metal_stdlib>
using namespace metal;

struct DrawUniforms {
    float4 rect;
    float4 fill;
    float4 stroke;
    float4 params;
    float4 uv;
    float4 clip;
    float4 effect;
    float4 extras;
    float4 paintBox;
    float4 maskBox;
    float4 maskParams;
    float3x3 transform;
    float4 viewportScale;
};

struct Varyings {
    float4 position [[position]];
    float2 local;
    float2 source;
    float2 uv;
};

vertex Varyings quadVertex(uint vertexID [[vertex_id]], constant DrawUniforms &u [[buffer(0)]]) {
    float2 corner = float2(float(vertexID & 1), float(vertexID >> 1));
    float2 source = u.rect.xy + corner * u.rect.zw;
    float3 projected = u.transform * float3(source, 1.0);
    float2 world = projected.xy / max(projected.z, 0.000001);
    float2 pixels = world * u.viewportScale.z;
    Varyings out;
    out.position = float4(
        pixels.x / u.viewportScale.x * 2.0 - 1.0,
        1.0 - pixels.y / u.viewportScale.y * 2.0,
        0.0,
        1.0
    );
    out.local = corner * u.rect.zw;
    out.source = source;
    out.uv = u.uv.xy + corner * u.uv.zw;
    return out;
}

float roundedDistance(float2 point, float2 halfSize, float radius) {
    float r = min(radius, min(halfSize.x, halfSize.y));
    float2 q = abs(point) - halfSize + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

float shapeCoverage(float distance, float scale) {
    float edge = max(fwidth(distance), 1.0 / max(scale, 0.001));
    return 1.0 - smoothstep(-edge, edge, distance);
}

float clipCoverage(float2 pixelPosition, constant DrawUniforms &u) {
    float2 point = pixelPosition / u.viewportScale.z;
    float2 center = (u.clip.xy + u.clip.zw) * 0.5;
    float2 halfSize = (u.clip.zw - u.clip.xy) * 0.5;
    if (halfSize.x <= 0.0 || halfSize.y <= 0.0) return 0.0;
    return shapeCoverage(roundedDistance(point - center, halfSize, u.effect.x), u.viewportScale.z);
}

float3 srgbToLinear(float3 value) {
    float3 low = value / 12.92;
    float3 high = pow((value + 0.055) / 1.055, float3(2.4));
    return select(high, low, value <= 0.04045);
}

float gradientPosition(float2 source, float4 box, float kind, float angle) {
    float2 center = box.xy + box.zw * 0.5;
    float2 local = source - center;
    if (kind > 1.5) {
        float degrees = atan2(local.x, -local.y) * 180.0 / M_PI_F;
        return fract((degrees - angle) / 360.0);
    }
    if (kind > 0.5) {
        return length(local) / max(length(box.zw) * 0.5, 0.000001);
    }
    float radians = angle * M_PI_F / 180.0;
    float2 direction = float2(sin(radians), -cos(radians));
    float extent = max(abs(box.z * direction.x) + abs(box.w * direction.y), 0.000001);
    return dot(local, direction / extent) + 0.5;
}

float4 paintColor(
    float4 solid,
    float gradientKind,
    float angle,
    float2 source,
    float4 box,
    texture2d<float> gradient,
    sampler sampleState
) {
    if (gradientKind >= 0.0) {
        float position = clamp(gradientPosition(source, box, gradientKind, angle), 0.0, 1.0);
        float4 color = gradient.sample(sampleState, float2(position, 0.5));
        color.a *= solid.a;
        return float4(color.rgb * color.a, color.a);
    }
    return float4(srgbToLinear(solid.rgb) * solid.a, solid.a);
}

float groupMask(
    float2 source,
    constant DrawUniforms &u,
    texture2d<float> maskTexture,
    sampler sampleState
) {
    if (u.maskParams.x < 0.5) return 1.0;
    if (source.x < u.maskBox.x || source.y < u.maskBox.y
        || source.x > u.maskBox.x + u.maskBox.z || source.y > u.maskBox.y + u.maskBox.w) {
        return 0.0;
    }
    if (u.maskParams.y >= 0.0) {
        float position = clamp(
            gradientPosition(source, u.maskBox, u.maskParams.y, u.maskParams.z),
            0.0,
            1.0
        );
        return maskTexture.sample(sampleState, float2(position, 0.5)).a;
    }
    return u.maskParams.w;
}

float4 adjustColor(float4 color, float saturation, float brightness) {
    if (color.a <= 0.0) return color;
    float3 rgb = color.rgb / color.a;
    float luminance = dot(rgb, float3(0.2126, 0.7152, 0.0722));
    rgb = clamp(mix(float3(luminance), rgb, saturation) * brightness, 0.0, 1.0);
    return float4(rgb * color.a, color.a);
}

fragment float4 quadFragment(
    Varyings in [[stage_in]],
    constant DrawUniforms &u [[buffer(0)]],
    texture2d<float> texture [[texture(0)]],
    texture2d<float> gradient [[texture(1)]],
    texture2d<float> strokeGradient [[texture(2)]],
    texture2d<float> maskTexture [[texture(3)]],
    sampler sampleState [[sampler(0)]]
) {
    int mode = int(round(u.effect.w));
    float2 halfSize = u.rect.zw * 0.5;
    float distance = roundedDistance(in.local - halfSize, halfSize, u.params.x);
    float coverage = shapeCoverage(distance, u.viewportScale.z);
    float4 output = float4(0.0);

    if (mode == 0) {
        output = paintColor(u.fill, u.effect.y, u.effect.z, in.source, u.paintBox, gradient, sampleState) * coverage;
        if (u.params.y > 0.0) {
            float band = abs(distance - u.params.z);
            float strokeCoverage = 1.0 - smoothstep(
                u.params.y - 1.0 / u.viewportScale.z,
                u.params.y + 1.0 / u.viewportScale.z,
                band
            );
            float4 strokeColor = paintColor(
                u.stroke,
                u.stroke.w < 0.0 ? -1.0 : u.effect.y,
                u.effect.z,
                in.source,
                u.paintBox,
                strokeGradient,
                sampleState
            );
            output = strokeColor * strokeCoverage + output * (1.0 - strokeColor.a * strokeCoverage);
        }
        if (u.extras.z > 0.0) {
            float cell = max(u.extras.w, 0.001);
            uint2 grid = uint2(floor(in.local / cell));
            uint hash = grid.x * 1664525u + grid.y * 1013904223u;
            hash ^= hash >> 16;
            float noise = float(hash) / 4294967296.0 - 0.5;
            float alpha = abs(noise) * u.extras.z * coverage;
            float shade = noise > 0.0 ? 1.0 : 0.0;
            float4 grain = float4(float3(shade) * alpha, alpha);
            output = grain + output * (1.0 - grain.a);
        }
    } else if (mode == 1) {
        if (u.extras.x > 0.5 && (any(in.uv < 0.0) || any(in.uv > 1.0))) {
            output = float4(0.0);
        } else {
            output = texture.sample(sampleState, in.uv) * u.params.w * coverage;
        }
    } else if (mode == 2) {
        float alpha = texture.sample(sampleState, in.uv).r;
        output = paintColor(u.fill, u.effect.y, u.effect.z, in.source, u.paintBox, gradient, sampleState) * alpha;
    } else if (mode == 3) {
        output = adjustColor(texture.sample(sampleState, in.uv), u.extras.z, u.extras.w)
            * u.params.w * coverage;
    } else if (mode == 4) {
        float blur = max(u.extras.y, 0.001);
        float shadowCoverage = 1.0 - smoothstep(-blur, blur, distance);
        output = paintColor(u.fill, -1.0, 0.0, in.source, u.paintBox, gradient, sampleState)
            * shadowCoverage;
    } else {
        float2 shifted = in.local - halfSize - u.extras.zw;
        float hole = roundedDistance(shifted, halfSize, u.params.x);
        float inverse = smoothstep(-max(u.extras.y, 0.001), max(u.extras.y, 0.001), hole);
        output = paintColor(u.fill, -1.0, 0.0, in.source, u.paintBox, gradient, sampleState)
            * inverse * coverage;
    }

    output *= groupMask(in.source, u, maskTexture, sampleState);
    output *= clipCoverage(in.position.xy, u);
    return output;
}
"""#
#endif
