import CoreGraphics
import CoreText
import Foundation
import ImageIO

struct AssetError: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

let root = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
let assets = root.appendingPathComponent("assets/branding")
let cache = root.appendingPathComponent(".cache/branding-assets")
let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)!
let inscription = "#dfaa60"
let density = "#38271f"

func number(_ value: CGFloat) -> String {
    let text = String(format: "%.4f", locale: Locale(identifier: "en_US_POSIX"), Double(value))
    return text.replacingOccurrences(of: #"\.?0+$"#, with: "", options: .regularExpression)
}

func color(_ hex: String) -> CGColor {
    let value = UInt32(hex.dropFirst(), radix: 16)!
    return CGColor(
        colorSpace: colorSpace,
        components: [
            CGFloat((value >> 16) & 255) / 255,
            CGFloat((value >> 8) & 255) / 255,
            CGFloat(value & 255) / 255, 1,
        ])!
}

func svgPath(_ path: CGPath) -> String {
    var parts: [String] = []
    path.applyWithBlock { pointer in
        let element = pointer.pointee
        let p = element.points
        func point(_ index: Int) -> String { "\(number(p[index].x)) \(number(p[index].y))" }
        switch element.type {
        case .moveToPoint: parts.append("M\(point(0))")
        case .addLineToPoint: parts.append("L\(point(0))")
        case .addQuadCurveToPoint: parts.append("Q\(point(0)) \(point(1))")
        case .addCurveToPoint: parts.append("C\(point(0)) \(point(1)) \(point(2))")
        case .closeSubpath: parts.append("Z")
        @unknown default: fatalError("Unsupported CoreGraphics path element")
        }
    }
    return parts.joined()
}

func write(_ data: Data, _ path: String) throws {
    let url = assets.appendingPathComponent(path)
    try FileManager.default.createDirectory(
        at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
    try data.write(to: url)
}

func rasterPNG(width: Int, height: Int, units: CGSize, draw: (CGContext) -> Void) throws -> Data {
    guard
        let context = CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
            space: colorSpace, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )
    else { throw AssetError("Cannot allocate branding bitmap") }
    context.translateBy(x: 0, y: CGFloat(height))
    context.scaleBy(x: CGFloat(width) / units.width, y: -CGFloat(height) / units.height)
    draw(context)
    guard let image = context.makeImage() else { throw AssetError("Cannot create branding image") }
    let output = NSMutableData()
    guard
        let destination = CGImageDestinationCreateWithData(output, "public.png" as CFString, 1, nil)
    else {
        throw AssetError("Cannot create PNG encoder")
    }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else { throw AssetError("Cannot encode PNG") }
    return output as Data
}

struct Icon {
    let size: CGFloat
    let corner: CGFloat
    let mark: CGPath

    func svg(small: Bool) -> String {
        """
        <svg xmlns="http://www.w3.org/2000/svg" width="\(number(size))" height="\(number(size))" viewBox="0 0 \(number(size)) \(number(size))" role="img" aria-labelledby="title desc">
          <title id="title">Antitone\(small ? " — small icon" : " — app icon")</title>
          <desc id="desc">An orange film-base tile with a dark rebate, an open window and an upward-pointing index\(small ? ", optically adjusted for small sizes" : "").</desc>
          <defs><linearGradient id="film-base" x1="0" y1="0" x2="1" y2="1"><stop stop-color="#d9874c"/><stop offset=".5" stop-color="#cb713f"/><stop offset="1" stop-color="#b85d36"/></linearGradient></defs>
          <rect width="\(number(size))" height="\(number(size))" rx="\(number(corner))" fill="url(#film-base)"/>
          <path fill="\(density)" fill-rule="evenodd" d="\(svgPath(mark))"/>
        </svg>
        """ + "\n"
    }

    func png(pixels: Int, macOS: Bool = false) throws -> Data {
        try rasterPNG(width: pixels, height: pixels, units: CGSize(width: size, height: size)) {
            context in
            if macOS {
                // A platform canvas inset aligns the tile with neighbouring Dock icons.
                let inset = size * 0.08
                context.translateBy(x: inset, y: inset)
                context.scaleBy(x: 0.84, y: 0.84)
            }
            context.saveGState()
            context.addPath(
                CGPath(
                    roundedRect: CGRect(x: 0, y: 0, width: size, height: size), cornerWidth: corner,
                    cornerHeight: corner, transform: nil))
            context.clip()
            let gradient = CGGradient(
                colorsSpace: colorSpace,
                colors: [color("#d9874c"), color("#cb713f"), color("#b85d36")] as CFArray,
                locations: [0, 0.5, 1])!
            context.drawLinearGradient(
                gradient, start: .zero, end: CGPoint(x: size, y: size), options: [])
            context.restoreGState()
            context.setFillColor(color(density))
            context.addPath(mark)
            context.fillPath(using: .evenOdd)
        }
    }
}

func largeIcon() -> Icon {
    let p = CGMutablePath()
    p.move(to: CGPoint(x: 26, y: 22))
    p.addLine(to: CGPoint(x: 102, y: 22))
    p.addQuadCurve(to: CGPoint(x: 106, y: 26), control: CGPoint(x: 106, y: 22))
    p.addLine(to: CGPoint(x: 106, y: 102))
    p.addQuadCurve(to: CGPoint(x: 102, y: 106), control: CGPoint(x: 106, y: 106))
    p.addLine(to: CGPoint(x: 26, y: 106))
    p.addQuadCurve(to: CGPoint(x: 22, y: 102), control: CGPoint(x: 22, y: 106))
    p.addLine(to: CGPoint(x: 22, y: 26))
    p.addQuadCurve(to: CGPoint(x: 26, y: 22), control: CGPoint(x: 22, y: 22))
    p.closeSubpath()
    p.move(to: CGPoint(x: 49, y: 34))
    p.addLine(to: CGPoint(x: 93, y: 34))
    p.addQuadCurve(to: CGPoint(x: 94, y: 35), control: CGPoint(x: 94, y: 34))
    p.addLine(to: CGPoint(x: 94, y: 93))
    p.addQuadCurve(to: CGPoint(x: 93, y: 94), control: CGPoint(x: 94, y: 94))
    p.addLine(to: CGPoint(x: 49, y: 94))
    p.addQuadCurve(to: CGPoint(x: 48, y: 93), control: CGPoint(x: 48, y: 94))
    p.addLine(to: CGPoint(x: 48, y: 35))
    p.addQuadCurve(to: CGPoint(x: 49, y: 34), control: CGPoint(x: 48, y: 34))
    p.closeSubpath()
    p.move(to: CGPoint(x: 35, y: 50))
    p.addLine(to: CGPoint(x: 29, y: 72))
    p.addLine(to: CGPoint(x: 41, y: 72))
    p.closeSubpath()
    return Icon(size: 128, corner: 28, mark: p)
}

func smallIcon() -> Icon {
    let p = CGMutablePath()
    p.move(to: CGPoint(x: 6, y: 5))
    p.addLine(to: CGPoint(x: 26, y: 5))
    p.addQuadCurve(to: CGPoint(x: 27, y: 6), control: CGPoint(x: 27, y: 5))
    p.addLine(to: CGPoint(x: 27, y: 26))
    p.addQuadCurve(to: CGPoint(x: 26, y: 27), control: CGPoint(x: 27, y: 27))
    p.addLine(to: CGPoint(x: 6, y: 27))
    p.addQuadCurve(to: CGPoint(x: 5, y: 26), control: CGPoint(x: 5, y: 27))
    p.addLine(to: CGPoint(x: 5, y: 6))
    p.addQuadCurve(to: CGPoint(x: 6, y: 5), control: CGPoint(x: 5, y: 5))
    p.closeSubpath()
    p.addRect(CGRect(x: 12, y: 8, width: 12, height: 16))
    p.move(to: CGPoint(x: 8.5, y: 12))
    p.addLine(to: CGPoint(x: 6.5, y: 18))
    p.addLine(to: CGPoint(x: 10.5, y: 18))
    p.closeSubpath()
    return Icon(size: 32, corner: 7, mark: p)
}

func wordmark() throws -> (CGPath, CGSize) {
    let url = assets.appendingPathComponent("fonts/Arimo-Bold.ttf")
    guard
        let descriptors = CTFontManagerCreateFontDescriptorsFromURL(url as CFURL)
            as? [CTFontDescriptor], let descriptor = descriptors.first
    else {
        throw AssetError("Cannot read checked-in Arimo Bold")
    }
    let font = CTFontCreateWithFontDescriptor(descriptor, 16, nil)
    guard CTFontCopyPostScriptName(font) as String == "Arimo-Bold" else {
        throw AssetError("Unexpected wordmark font")
    }
    let tracking: CGFloat = 16 * 0.08
    let line = CTLineCreateWithAttributedString(
        NSAttributedString(
            string: "ANTITONE",
            attributes: [
                NSAttributedString.Key(kCTFontAttributeName as String): font,
                NSAttributedString.Key(kCTKernAttributeName as String): tracking,
            ]))
    let cap = CTFontGetCapHeight(font)
    let triangleWidth = cap * 22 / 12
    let textStart = triangleWidth + 16 * 0.45
    let baseline: CGFloat = 13
    let path = CGMutablePath()
    path.move(to: CGPoint(x: 0, y: baseline - cap))
    path.addLine(to: CGPoint(x: triangleWidth, y: baseline - cap / 2))
    path.addLine(to: CGPoint(x: 0, y: baseline))
    path.closeSubpath()
    var textWidth: CGFloat = 0
    for run in CTLineGetGlyphRuns(line) as! [CTRun] {
        let count = CTRunGetGlyphCount(run)
        var glyphs = [CGGlyph](repeating: 0, count: count)
        var positions = [CGPoint](repeating: .zero, count: count)
        var advances = [CGSize](repeating: .zero, count: count)
        CTRunGetGlyphs(run, CFRange(), &glyphs)
        CTRunGetPositions(run, CFRange(), &positions)
        CTFontGetAdvancesForGlyphs(font, .horizontal, glyphs, &advances, count)
        for i in 0..<count {
            guard let glyph = CTFontCreatePathForGlyph(font, glyphs[i], nil) else {
                throw AssetError("Missing wordmark glyph")
            }
            let transform = CGAffineTransform(
                a: 1, b: 0, c: 0, d: -1, tx: textStart + positions[i].x,
                ty: baseline - positions[i].y)
            path.addPath(glyph, transform: transform)
            textWidth = max(textWidth, positions[i].x + advances[i].width)
        }
    }
    return (path, CGSize(width: textStart + textWidth, height: 16))
}

func appendLE(_ value: UInt32, bytes: Int, to data: inout Data) {
    for i in 0..<bytes { data.append(UInt8((value >> (8 * i)) & 255)) }
}

func ico(_ images: [(Int, Data)]) -> Data {
    var data = Data()
    appendLE(0, bytes: 2, to: &data)
    appendLE(1, bytes: 2, to: &data)
    appendLE(UInt32(images.count), bytes: 2, to: &data)
    var offset = 6 + 16 * images.count
    for (size, image) in images {
        data.append(size == 256 ? 0 : UInt8(size))
        data.append(size == 256 ? 0 : UInt8(size))
        data.append(0)
        data.append(0)
        appendLE(1, bytes: 2, to: &data)
        appendLE(32, bytes: 2, to: &data)
        appendLE(UInt32(image.count), bytes: 4, to: &data)
        appendLE(UInt32(offset), bytes: 4, to: &data)
        offset += image.count
    }
    for (_, image) in images { data.append(image) }
    return data
}

func generate() throws {
    try FileManager.default.createDirectory(at: cache, withIntermediateDirectories: true)
    let large = largeIcon()
    let small = smallIcon()
    try write(Data(large.svg(small: false).utf8), "icon.svg")
    try write(Data(small.svg(small: true).utf8), "icon-small.svg")
    var icoImages: [(Int, Data)] = []
    for size in [16, 24, 32, 48, 64, 128, 256, 512, 1024] {
        let image = try (size <= 32 ? small : large).png(pixels: size)
        try write(image, "png/icon-\(size).png")
        if size <= 256 { icoImages.append((size, image)) }
    }
    try write(ico(icoImages), "icon.ico")
    let iconset = cache.appendingPathComponent("Antitone.iconset")
    try FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
    for points in [16, 32, 128, 256, 512] {
        for scale in [1, 2] {
            let image = try (points <= 32 ? small : large).png(pixels: points * scale, macOS: true)
            try image.write(
                to: iconset.appendingPathComponent(
                    "icon_\(points)x\(points)\(scale == 2 ? "@2x" : "").png"))
        }
    }
    let output = assets.appendingPathComponent("icon.icns")
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
    process.arguments = ["-c", "icns", iconset.path, "-o", output.path]
    try process.run()
    process.waitUntilExit()
    guard process.terminationStatus == 0 else { throw AssetError("iconutil failed") }
    let (path, size) = try wordmark()
    for (name, fill) in [("wordmark-amber", inscription), ("wordmark-density", density)] {
        let svg = """
            <svg xmlns="http://www.w3.org/2000/svg" width="\(number(size.width))" height="16" viewBox="0 0 \(number(size.width)) 16" role="img" aria-labelledby="title desc">
              <title id="title">Antitone</title>
              <desc id="desc">A leading right-pointing film index followed by ANTITONE in outlined Arimo Bold lettering.</desc>
              <path fill="\(fill)" d="\(svgPath(path))"/>
            </svg>
            """ + "\n"
        try write(Data(svg.utf8), "\(name).svg")
    }
    let width = Int(ceil(size.width * 4))
    let image = try rasterPNG(
        width: width, height: 64, units: CGSize(width: CGFloat(width) / 4, height: 16)
    ) { context in
        context.setFillColor(color(inscription))
        context.addPath(path)
        context.fillPath()
    }
    try write(image, "wordmark-header.png")
    print("Generated branding assets; header wordmark \(number(size.width)) × 16 logical points.")
}

do { try generate() } catch {
    fputs("Branding generation failed: \(error)\n", stderr)
    exit(1)
}
