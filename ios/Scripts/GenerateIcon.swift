import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

// Render a vector monogram without font or window-server dependencies.
let side = 1024
let context = CGContext(data: nil, width: side, height: side, bitsPerComponent: 8,
                        bytesPerRow: side * 4, space: CGColorSpaceCreateDeviceRGB(),
                        bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
context.setFillColor(CGColor(red: 0.73, green: 0.28, blue: 0.08, alpha: 1))
context.fill(CGRect(x: 0, y: 0, width: side, height: side))
let points: [CGPoint] = [(190, 725), (650, 725), (650, 610), (355, 340), (650, 340),
                         (650, 225), (190, 225), (190, 340), (485, 610), (190, 610)].map { CGPoint(x: $0.0, y: $0.1) }
context.setFillColor(CGColor(gray: 1, alpha: 1))
context.addLines(between: points)
context.closePath()
context.fillPath()
context.fillEllipse(in: CGRect(x: 710, y: 225, width: 125, height: 125))
let output = URL(fileURLWithPath: "Sources/Assets.xcassets/AppIcon.appiconset/Icon.png")
try FileManager.default.createDirectory(at: output.deletingLastPathComponent(), withIntermediateDirectories: true)
let destination = CGImageDestinationCreateWithURL(output as CFURL, UTType.png.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(destination, context.makeImage()!, nil)
guard CGImageDestinationFinalize(destination) else { fatalError("Could not write icon") }
