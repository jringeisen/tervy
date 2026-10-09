// External visibility metadata only; no screenshot or window-title collection.
import AppKit
import CoreGraphics
import Foundation
let pid = Int32(CommandLine.arguments[1])!
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
let visible = windows.contains { ($0[kCGWindowOwnerPID as String] as? Int32) == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }
let result: [String: Any] = ["on_screen": visible, "frontmost": NSWorkspace.shared.frontmostApplication?.processIdentifier == pid, "verified": true]
print(String(data: try JSONSerialization.data(withJSONObject: result), encoding: .utf8)!)
