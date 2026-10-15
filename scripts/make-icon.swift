// Renders Resources/AppIcon.icns. Run: swift scripts/make-icon.swift
import AppKit

func render(size: CGFloat) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(size), pixelsHigh: Int(size),
                               bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let s = size / 1024

    // Squircle background inside the standard macOS icon grid.
    let rect = NSRect(x: 100 * s, y: 100 * s, width: 824 * s, height: 824 * s)
    let background = NSBezierPath(roundedRect: rect, xRadius: 185 * s, yRadius: 185 * s)
    NSGradient(colors: [NSColor(red: 0.07, green: 0.62, blue: 0.72, alpha: 1),
                        NSColor(red: 0.20, green: 0.27, blue: 0.70, alpha: 1)])!
        .draw(in: background, angle: -60)

    // Stacked database discs.
    let white = NSColor.white
    let cx = 512 * s, w = 420 * s, h = 120 * s
    for (i, y) in [330.0, 450.0, 570.0].enumerated() {
        let body = NSRect(x: cx - w / 2, y: CGFloat(y) * s, width: w, height: 110 * s)
        let path = NSBezierPath()
        path.appendRect(NSRect(x: body.minX, y: body.minY + h / 2, width: w, height: body.height - h / 2 + 1))
        path.appendOval(in: NSRect(x: body.minX, y: body.minY, width: w, height: h))
        white.withAlphaComponent(i == 2 ? 1 : 0.86).setFill()
        path.fill()
        // Top rim of each disc.
        let top = NSBezierPath(ovalIn: NSRect(x: body.minX, y: body.maxY - h / 2, width: w, height: h))
        (i == 2 ? NSColor(red: 0.85, green: 0.95, blue: 1, alpha: 1) : white).setFill()
        top.fill()
        NSColor(red: 0.13, green: 0.40, blue: 0.70, alpha: 0.35).setStroke()
        top.lineWidth = 6 * s
        top.stroke()
    }

    // A small kite.
    let kite = NSBezierPath()
    kite.move(to: NSPoint(x: 770 * s, y: 860 * s))
    kite.line(to: NSPoint(x: 830 * s, y: 800 * s))
    kite.line(to: NSPoint(x: 770 * s, y: 690 * s))
    kite.line(to: NSPoint(x: 710 * s, y: 800 * s))
    kite.close()
    NSColor(red: 1, green: 0.75, blue: 0.2, alpha: 1).setFill()
    kite.fill()
    NSGraphicsContext.restoreGraphicsState()
    return rep
}

let root = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "Resources")
let iconset = root.appendingPathComponent("AppIcon.iconset")
try? FileManager.default.removeItem(at: iconset)
try! FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
for base in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let name = scale == 1 ? "icon_\(base)x\(base).png" : "icon_\(base)x\(base)@2x.png"
        let data = render(size: CGFloat(base * scale)).representation(using: .png, properties: [:])!
        try! data.write(to: iconset.appendingPathComponent(name))
    }
}
let task = Process()
task.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
task.arguments = ["-c", "icns", iconset.path, "-o", root.appendingPathComponent("AppIcon.icns").path]
try! task.run()
task.waitUntilExit()
try? FileManager.default.removeItem(at: iconset)
print("Wrote \(root.appendingPathComponent("AppIcon.icns").path)")
