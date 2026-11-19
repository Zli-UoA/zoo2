#if DEBUG
import CoreText
import ImageIO
import Foundation

enum PreviewPDF {
    static let data: Data = {
        let bytes = NSMutableData()
        var bounds = CGRect(x: 0, y: 0, width: 595, height: 842)
        guard let consumer = CGDataConsumer(data: bytes),
              let context = CGContext(consumer: consumer, mediaBox: &bounds, nil) else { return Data() }
        let attributes = [kCTFontAttributeName: CTFontCreateWithName("HiraginoSans-W3" as CFString, 24, nil)] as CFDictionary
        for page in 1...2 {
            context.beginPDFPage(nil)
            context.textPosition = CGPoint(x: 60, y: 720)
            let text = CFAttributedStringCreate(nil, "PDFの表示確認（\(page)ページ目）" as CFString, attributes)!
            CTLineDraw(CTLineCreateWithAttributedString(text), context)
            context.endPDFPage()
        }
        context.closePDF()
        return bytes as Data
    }()
}

enum PreviewImage {
    static func data(for path: String) -> Data? {
        switch path {
        case "/stamp.png": stamp
        case "/tall.png": tall
        case "/wide.png": wide
        default: nil
        }
    }

    private static let stamp = Data(base64Encoded: "iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAYAAABzenr0AAAAeUlEQVR4nO3T0QmAMAyE4W7mNI7lVK7hDu0AtvaO5FqCOcib+H8glpKLuOe86ui2xuUIJC5F/A/ABF1BnmEaooxPESvin4hQgHofr1sG6MVZRAJMf4IVMIyjCAtgGrcg3OLop2AvAQmIBfBG0HEUgT5jGvpSSTynXgP20vIB9b499AAAAABJRU5ErkJggg==")
    private static let tall = stripedImage(width: 240, height: 1600)
    private static let wide = stripedImage(width: 1600, height: 400)

    private static func stripedImage(width: Int, height: Int) -> Data? {
        guard let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8,
                                      bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        for row in 0..<8 {
            context.setFillColor(CGColor(red: CGFloat(row) / 10, green: 0.5, blue: 0.8, alpha: 1))
            context.fill(CGRect(x: 0, y: row * height / 8, width: width, height: height / 8))
        }
        let bytes = NSMutableData()
        guard let image = context.makeImage(),
              let destination = CGImageDestinationCreateWithData(bytes, "public.png" as CFString, 1, nil) else { return nil }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return bytes as Data
    }
}

// 可変な共有状態を持たず、UI テスト専用画像を同期で返して外部配信に依存させない。
final class PreviewEmojiURLProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool {
        request.url?.host == "ui.test.invalid" && ["/stamp.png", "/tall.png", "/wide.png"].contains(request.url?.path ?? "")
    }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        guard let url = request.url,
              let response = HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil,
                                             headerFields: ["Content-Type": "image/png"]),
              let data = PreviewImage.data(for: url.path) else { return }
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
#endif
