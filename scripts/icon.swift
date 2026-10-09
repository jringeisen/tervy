// Original DevDesk app icon, generated locally with Apple's drawing APIs.
import AppKit
let side: CGFloat = 1024
let image = NSImage(size: NSSize(width: side, height: side))
image.lockFocus()
NSColor(calibratedRed: 0.07, green: 0.10, blue: 0.17, alpha: 1).setFill()
NSBezierPath(roundedRect: NSRect(x: 0, y: 0, width: side, height: side), xRadius: 200, yRadius: 200).fill()
NSColor(calibratedRed: 0.12, green: 0.18, blue: 0.29, alpha: 1).setFill()
NSBezierPath(roundedRect: NSRect(x: 115, y: 180, width: 794, height: 664), xRadius: 50, yRadius: 50).fill()
NSColor(calibratedRed: 0.27, green: 0.60, blue: 0.98, alpha: 1).setFill()
NSRect(x: 145, y: 210, width: 165, height: 604).fill()
let text: NSString = ">_"
text.draw(at: NSPoint(x: 355, y: 350), withAttributes: [.font: NSFont.monospacedSystemFont(ofSize: 245, weight: .medium), .foregroundColor: NSColor.white])
image.unlockFocus()
let bitmap = NSBitmapImageRep(data: image.tiffRepresentation!)!
try bitmap.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
