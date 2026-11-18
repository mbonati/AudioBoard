// The site's video encoder, for website/tools/site_media.rs (macOS has no
// ffmpeg, but AVFoundation does H.264 and AAC). Compiled on demand by that
// tool into target/site-media/encode. Two commands:
//
//   encode video W H FPS OUT.mp4   raw BGRA frames (W×H×4 bytes each) on
//                                  stdin → H.264, no sound
//   encode mux VIDEO.mp4 AUDIO.m4a OUT.mp4
//                                  both tracks copied into one file, ready
//                                  to stream (moov first)
import AVFoundation
import Foundation

func fail(_ msg: String) -> Never {
    FileHandle.standardError.write((msg + "\n").data(using: .utf8)!)
    exit(1)
}

func readFully(_ count: Int) -> Data? {
    var data = Data(capacity: count)
    while data.count < count {
        let chunk = FileHandle.standardInput.readData(ofLength: count - data.count)
        if chunk.isEmpty { return nil }
        data.append(chunk)
    }
    return data
}

func video(_ w: Int, _ h: Int, _ fps: Int32, _ out: URL) {
    try? FileManager.default.removeItem(at: out)
    guard let writer = try? AVAssetWriter(outputURL: out, fileType: .mp4) else { fail("cannot write \(out.path)") }
    let settings: [String: Any] = [
        AVVideoCodecKey: AVVideoCodecType.h264,
        AVVideoWidthKey: w,
        AVVideoHeightKey: h,
        AVVideoCompressionPropertiesKey: [
            AVVideoAverageBitRateKey: Int(ProcessInfo.processInfo.environment["BITRATE"] ?? "") ?? 3_000_000,
            AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel,
            AVVideoMaxKeyFrameIntervalKey: Int(fps) * 2,
            AVVideoAllowFrameReorderingKey: true,
        ],
        AVVideoColorPropertiesKey: [
            AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_709_2,
            AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_709_2,
            AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_709_2,
        ],
    ]
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: settings)
    input.expectsMediaDataInRealTime = false
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey as String: w,
        kCVPixelBufferHeightKey as String: h,
    ])
    writer.add(input)
    guard writer.startWriting() else { fail("startWriting: \(String(describing: writer.error))") }
    writer.startSession(atSourceTime: .zero)
    let frameBytes = w * h * 4
    var n: Int64 = 0
    while let frame = readFully(frameBytes) {
        while !input.isReadyForMoreMediaData { usleep(2000) }
        guard let pool = adaptor.pixelBufferPool else { fail("no pixel buffer pool") }
        var pb: CVPixelBuffer?
        CVPixelBufferPoolCreatePixelBuffer(nil, pool, &pb)
        guard let buf = pb else { fail("no pixel buffer") }
        CVPixelBufferLockBaseAddress(buf, [])
        let dst = CVPixelBufferGetBaseAddress(buf)!.assumingMemoryBound(to: UInt8.self)
        let stride = CVPixelBufferGetBytesPerRow(buf)
        frame.withUnsafeBytes { (src: UnsafeRawBufferPointer) in
            for y in 0..<h {
                memcpy(dst + y * stride, src.baseAddress! + y * w * 4, w * 4)
            }
        }
        CVPixelBufferUnlockBaseAddress(buf, [])
        if !adaptor.append(buf, withPresentationTime: CMTime(value: n, timescale: fps)) {
            fail("append frame \(n): \(String(describing: writer.error))")
        }
        n += 1
    }
    input.markAsFinished()
    let done = DispatchSemaphore(value: 0)
    writer.finishWriting { done.signal() }
    done.wait()
    if writer.status != .completed { fail("finish: \(String(describing: writer.error))") }
    print("encoded \(n) frames → \(out.path)")
}

func mux(_ videoURL: URL, _ audioURL: URL, _ out: URL) {
    try? FileManager.default.removeItem(at: out)
    let comp = AVMutableComposition()
    let v = AVURLAsset(url: videoURL)
    let a = AVURLAsset(url: audioURL)
    let sem = DispatchSemaphore(value: 0)
    var vTrack: AVAssetTrack?
    var aTrack: AVAssetTrack?
    var vDur = CMTime.zero
    Task {
        vTrack = try? await v.loadTracks(withMediaType: .video).first
        aTrack = try? await a.loadTracks(withMediaType: .audio).first
        vDur = (try? await v.load(.duration)) ?? .zero
        sem.signal()
    }
    sem.wait()
    guard let vt = vTrack, let at = aTrack else { fail("missing a track") }
    let range = CMTimeRange(start: .zero, duration: vDur)
    let cv = comp.addMutableTrack(withMediaType: .video, preferredTrackID: kCMPersistentTrackID_Invalid)!
    let ca = comp.addMutableTrack(withMediaType: .audio, preferredTrackID: kCMPersistentTrackID_Invalid)!
    do {
        try cv.insertTimeRange(range, of: vt, at: .zero)
        try ca.insertTimeRange(range, of: at, at: .zero)
    } catch { fail("insert: \(error)") }
    guard let export = AVAssetExportSession(asset: comp, presetName: AVAssetExportPresetPassthrough) else { fail("no export session") }
    export.outputURL = out
    export.outputFileType = .mp4
    export.shouldOptimizeForNetworkUse = true
    let done = DispatchSemaphore(value: 0)
    export.exportAsynchronously { done.signal() }
    done.wait()
    if export.status != .completed { fail("export: \(String(describing: export.error))") }
    print("muxed → \(out.path)")
}

let args = CommandLine.arguments
switch args.count > 1 ? args[1] : "" {
case "video" where args.count == 6:
    video(Int(args[2])!, Int(args[3])!, Int32(args[4])!, URL(fileURLWithPath: args[5]))
case "mux" where args.count == 5:
    mux(URL(fileURLWithPath: args[2]), URL(fileURLWithPath: args[3]), URL(fileURLWithPath: args[4]))
default:
    fail("usage: encode video W H FPS OUT.mp4 | encode mux VIDEO AUDIO OUT.mp4")
}
