// Regenerate the desktop icon from the shared logo:
// swift native/macos/build-icon.swift
import AppKit

let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    .deletingLastPathComponent().deletingLastPathComponent()
let assets = root.appendingPathComponent("crates/acheflow-ui/assets")
let logo = NSImage(contentsOf: assets.appendingPathComponent("achedev-logo.svg"))!
let temporary = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
let iconset = temporary.appendingPathComponent("Acheflow.iconset")
try FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(at: temporary) }

for size in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let pixels = size * scale
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels,
            pixelsHigh: pixels, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
            isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
        NSGraphicsContext.current?.imageInterpolation = .high
        let side = CGFloat(pixels)
        // Preserve the logo's proportions with a little breathing room in the square tile.
        let height = side * 0.84
        let width = height * logo.size.width / logo.size.height
        logo.draw(in: NSRect(x: (side - width) / 2, y: (side - height) / 2,
            width: width, height: height), from: .zero, operation: .sourceOver, fraction: 1)
        NSGraphicsContext.restoreGraphicsState()
        let suffix = scale == 2 ? "@2x" : ""
        try bitmap.representation(using: .png, properties: [:])!.write(
            to: iconset.appendingPathComponent("icon_\(size)x\(size)\(suffix).png"))
    }
}
let tool = Process()
tool.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
tool.arguments = ["-c", "icns", iconset.path, "-o", assets.appendingPathComponent("achedev-logo.icns").path]
try tool.run()
tool.waitUntilExit()
guard tool.terminationStatus == 0 else { exit(tool.terminationStatus) }
