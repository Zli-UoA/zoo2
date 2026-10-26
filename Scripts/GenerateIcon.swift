import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

// zoo2 の frontend/public/icons/icon-512.png を原本として、ブラウザ版と同じ意匠を保つ。
let sourceURL = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    .appendingPathComponent("BrowserIcon.png")
guard let source = CGImageSourceCreateWithURL(sourceURL as CFURL, nil),
      let browserIcon = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
    fatalError("ブラウザ版のアイコンを読み込めませんでした。")
}
let side = 1024
let context = CGContext(data: nil, width: side, height: side, bitsPerComponent: 8,
                        bytesPerRow: side * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                        bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
context.interpolationQuality = .high
context.draw(browserIcon, in: CGRect(x: 0, y: 0, width: side, height: side))
let output = URL(fileURLWithPath: "Sources/Assets.xcassets/AppIcon.appiconset/Icon.png")
try FileManager.default.createDirectory(at: output.deletingLastPathComponent(), withIntermediateDirectories: true)
let destination = CGImageDestinationCreateWithURL(output as CFURL, UTType.png.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(destination, context.makeImage()!, nil)
guard CGImageDestinationFinalize(destination) else { fatalError("アイコンを書き出せませんでした。") }
