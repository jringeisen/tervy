// Render the canonical vector geometry; no independent raster logo source.
import AppKit
import Foundation
let source = URL(fileURLWithPath: CommandLine.arguments[1])
let brand = try JSONSerialization.jsonObject(with: Data(contentsOf: source)) as! [String: Any]
let colors = brand["colors"] as! [String: String]
let commands = brand["mark"] as! [[Any]]
func color(_ hex: String) -> NSColor {
    let value = UInt32(hex.dropFirst(), radix: 16)!
    return NSColor(srgbRed: CGFloat((value >> 16) & 255) / 255,
                   green: CGFloat((value >> 8) & 255) / 255,
                   blue: CGFloat(value & 255) / 255, alpha: 1)
}
let image = NSImage(size: NSSize(width: 1024, height: 1024))
image.lockFocus()
color(colors["graphite"]!).setFill()
NSBezierPath(roundedRect: NSRect(x: 0, y: 0, width: 1024, height: 1024), xRadius: 224, yRadius: 224).fill()
let path = NSBezierPath()
path.windingRule = .evenOdd
for command in commands {
    func point(_ index: Int) -> NSPoint {
        NSPoint(x: (command[index] as! NSNumber).doubleValue * 16,
                y: 1024 - (command[index + 1] as! NSNumber).doubleValue * 16)
    }
    switch command[0] as! String {
    case "M": path.move(to: point(1))
    case "L": path.line(to: point(1))
    case "C": path.curve(to: point(5), controlPoint1: point(1), controlPoint2: point(3))
    case "Z": path.close()
    default: fatalError("Unknown brand path command")
    }
}
color(colors["white"]!).setFill()
path.fill()
image.unlockFocus()
let bitmap = NSBitmapImageRep(data: image.tiffRepresentation!)!
try bitmap.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: CommandLine.arguments[2]))
