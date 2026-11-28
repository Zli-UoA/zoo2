import AVFoundation
import CoreImage
import CoreImage.CIFilterBuiltins
import SwiftUI
import UIKit

/// QR コードによる検証（docs/e2ee.md の「QR コードによる検証」）の QR の描画と読み取り。中身の作成と検証は zoo-crypto
/// （`SasVerification.view().qrCode`、`scanQrCode`）が行い、ここはバイト列と画像の変換だけを持つ。
/// バイトモード、誤り訂正は M。
enum QrCodeKit {
    /// QR の先頭（Matrix と同じ `MATRIX`）、版、長さの最小。形が合わないものを、暗号の検証の前に見分ける。
    private static let prefix: [UInt8] = [0x4d, 0x41, 0x54, 0x52, 0x49, 0x58]

    static func looksLikeVerificationQr(_ bytes: Data) -> Bool {
        let header = prefix.count + 4
        guard bytes.count >= header + 72, Array(bytes.prefix(prefix.count)) == prefix,
              bytes[bytes.startIndex + 6] == 0x02, bytes[bytes.startIndex + 7] == 0x00 else { return false }
        let txnLength = Int(bytes[bytes.startIndex + 8]) << 8 | Int(bytes[bytes.startIndex + 9])
        return bytes.count >= header + txnLength + 72
    }

    /// QR の画像。`scale` 倍の整数倍で、補間せずに拡大する（ぼやけると読み取れない）。
    static func image(for bytes: Data, scale: CGFloat = 8) -> UIImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = bytes
        filter.correctionLevel = "M"
        guard let output = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: scale, y: scale)) else { return nil }
        let context = CIContext()
        guard let cgImage = context.createCGImage(output, from: output.extent) else { return nil }
        return UIImage(cgImage: cgImage)
    }

    /// 画像から QR を読み、バイト列を返す。検証の QR の形をしていなければ `nil`。`CIDetector` が返す QR の記述子
    /// （誤り訂正後のコードワード）から、バイトモードの中身をそのまま取り出す（`messageString` は文字列なので、
    /// 0x00 や 0xFF を含むバイト列を壊す）。Vision の `VNDetectBarcodesRequest` はシミュレーターで使えない。
    static func decode(_ image: CIImage) -> Data? {
        let detector = CIDetector(ofType: CIDetectorTypeQRCode, context: nil, options: [CIDetectorAccuracy: CIDetectorAccuracyHigh])
        for case let feature as CIQRCodeFeature in detector?.features(in: image) ?? [] {
            if let descriptor = feature.symbolDescriptor, let payload = bytes(from: descriptor), looksLikeVerificationQr(payload) {
                return payload
            }
        }
        return nil
    }

    static func decode(_ image: CGImage) -> Data? { decode(CIImage(cgImage: image)) }

    static func decode(pixelBuffer: CVPixelBuffer) -> Data? {
        decode(CIImage(cvPixelBuffer: pixelBuffer).oriented(.right))
    }

    private static let alphanumeric = Array("0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:".utf8)

    /// QR の記述子の誤り訂正後のデータコードワードから、セグメントの中身を、バイト列として連結して返す。QR の作成側（CoreImage や
    /// 他の実装）は、先頭の `MATRIX` を英数字モード、続きをバイトモードのように、中身に合わせてモードを切り替えるので、
    /// 数字・英数字・バイトのモードを読む（どのモードでも、1 文字は 1 バイト）。漢字や ECI など、ほかのモードがあれば `nil`。
    /// ブロックが複数あるときのインターリーブは、記述子が解いた順で返ってくる。
    static func bytes(from descriptor: CIQRCodeDescriptor) -> Data? {
        let payload = [UInt8](descriptor.errorCorrectedPayload)
        var position = 0
        func read(_ count: Int) -> Int? {
            guard position + count <= payload.count * 8 else { return nil }
            var value = 0
            for _ in 0..<count {
                value = value << 1 | Int(payload[position / 8] >> (7 - UInt8(position % 8)) & 1)
                position += 1
            }
            return value
        }
        let version = descriptor.symbolVersion
        var result = Data()
        while let mode = read(4) {
            switch mode {
            case 0:
                return result.isEmpty ? nil : result  // 終端
            case 0b0001:  // 数字
                guard var remaining = read(version <= 9 ? 10 : version <= 26 ? 12 : 14) else { return nil }
                while remaining > 0 {
                    let digits = min(remaining, 3)
                    guard let value = read(digits == 3 ? 10 : digits == 2 ? 7 : 4) else { return nil }
                    result.append(contentsOf: String(format: "%0\(digits)d", value).utf8)
                    remaining -= digits
                }
            case 0b0010:  // 英数字
                guard var remaining = read(version <= 9 ? 9 : version <= 26 ? 11 : 13) else { return nil }
                while remaining > 0 {
                    if remaining >= 2 {
                        guard let value = read(11), value / 45 < 45 else { return nil }
                        result.append(alphanumeric[value / 45]); result.append(alphanumeric[value % 45])
                        remaining -= 2
                    } else {
                        guard let value = read(6), value < 45 else { return nil }
                        result.append(alphanumeric[value])
                        remaining -= 1
                    }
                }
            case 0b0100:  // バイト
                guard let length = read(version <= 9 ? 8 : 16) else { return nil }
                for _ in 0..<length {
                    guard let byte = read(8) else { return nil }
                    result.append(UInt8(byte))
                }
            default:
                return nil
            }
        }
        return result.isEmpty ? nil : result
    }
}

/// QR を表示する。
struct QrCodeImage: View {
    let bytes: Data
    var body: some View {
        if let image = QrCodeKit.image(for: bytes) {
            Image(uiImage: image)
                .interpolation(.none)
                .resizable()
                .scaledToFit()
                .frame(maxWidth: 260)
                .padding(12)
                .background(Color.white, in: RoundedRectangle(cornerRadius: 12))
                .accessibilityLabel("検証の QR コード")
        }
    }
}

/// カメラで QR を読む。見つけたバイト列（検証の QR の形のもの）を `onScan` に渡す。カメラを使えないときは `onUnavailable`。
struct QrScannerView: UIViewControllerRepresentable {
    let onScan: (Data) -> Void
    let onUnavailable: () -> Void

    func makeUIViewController(context: Context) -> ScannerController {
        let controller = ScannerController()
        controller.onScan = onScan
        controller.onUnavailable = onUnavailable
        return controller
    }

    func updateUIViewController(_ controller: ScannerController, context: Context) {}

    static func dismantleUIViewController(_ controller: ScannerController, coordinator: ()) { controller.stop() }

    final class ScannerController: UIViewController {
        var onScan: ((Data) -> Void)?
        var onUnavailable: (() -> Void)?
        private let session = AVCaptureSession()
        private let queue = DispatchQueue(label: "works.zli.zoo.qr-scan")
        private var preview: AVCaptureVideoPreviewLayer?
        private var finished = false
        private lazy var frames = FrameDelegate { [weak self] payload in
            DispatchQueue.main.async {
                guard let self, !self.finished else { return }
                self.onScan?(payload)
            }
        }

        override func viewDidLoad() {
            super.viewDidLoad()
            view.backgroundColor = .black
            Task { @MainActor in await configure() }
        }

        override func viewDidLayoutSubviews() {
            super.viewDidLayoutSubviews()
            preview?.frame = view.bounds
        }

        private func configure() async {
            let granted = await AVCaptureDevice.requestAccess(for: .video)
            // 許可を待つあいだに、画面が取り除かれて止められていたら（相手が検証を中止したなど）、何も始めない。
            guard !finished else { return }
            guard granted,
                  let device = AVCaptureDevice.default(for: .video),
                  let input = try? AVCaptureDeviceInput(device: device), session.canAddInput(input) else {
                onUnavailable?()
                return
            }
            session.addInput(input)
            let output = AVCaptureVideoDataOutput()
            output.setSampleBufferDelegate(frames, queue: queue)
            output.alwaysDiscardsLateVideoFrames = true
            guard session.canAddOutput(output) else { onUnavailable?(); return }
            session.addOutput(output)
            let layer = AVCaptureVideoPreviewLayer(session: session)
            layer.videoGravity = .resizeAspectFill
            layer.frame = view.bounds
            view.layer.addSublayer(layer)
            preview = layer
            let session = session
            queue.async { session.startRunning() }
        }

        func stop() {
            finished = true
            let session = session
            queue.async { if session.isRunning { session.stopRunning() } }
        }
    }

    /// カメラのフレームを読む（キューで呼ばれるので、メインアクターには置かない）。
    final class FrameDelegate: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate, @unchecked Sendable {
        private let found: @Sendable (Data) -> Void
        private var lastScan = Date.distantPast
        init(found: @escaping @Sendable (Data) -> Void) { self.found = found }

        func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
            // 毎フレームは読まず、0.25 秒に 1 回にする。
            guard Date().timeIntervalSince(lastScan) > 0.25, let buffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
            lastScan = Date()
            if let payload = QrCodeKit.decode(pixelBuffer: buffer) { found(payload) }
        }
    }
}
