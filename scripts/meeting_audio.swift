// Native meeting audio worker. Screen frames are never received or saved.
import AVFoundation
import CoreGraphics
import Foundation
import ScreenCaptureKit
import Speech

final class AudioWriter {
    let url: URL
    private var file: AVAudioFile?
    private var failure: Error?

    init(url: URL) throws {
        self.url = url
    }

    func append(_ sample: CMSampleBuffer) {
        guard CMSampleBufferDataIsReady(sample) else { return }
        guard failure == nil else { return }
        do {
            guard let description = CMSampleBufferGetFormatDescription(sample) else {
                throw NSError(domain: "TervyMeeting", code: 8, userInfo: [NSLocalizedDescriptionKey: "Audio sample has no format"])
            }
            let format = AVAudioFormat(cmAudioFormatDescription: description)
            if file == nil {
                let settings: [String: Any] = [
                    AVFormatIDKey: kAudioFormatLinearPCM,
                    AVSampleRateKey: format.sampleRate,
                    AVNumberOfChannelsKey: format.channelCount,
                    AVLinearPCMBitDepthKey: 16,
                    AVLinearPCMIsFloatKey: false,
                    AVLinearPCMIsBigEndianKey: false,
                    AVLinearPCMIsNonInterleaved: false
                ]
                do {
                    file = try AVAudioFile(forWriting: url, settings: settings,
                                           commonFormat: format.commonFormat, interleaved: format.isInterleaved)
                } catch { throw NSError(domain: "TervyMeeting", code: 10, userInfo: [NSLocalizedDescriptionKey: "Open audio file: \(error); format: \(format)"]) }
            }
            let frames = CMSampleBufferGetNumSamples(sample)
            guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(frames)) else {
                throw NSError(domain: "TervyMeeting", code: 9, userInfo: [NSLocalizedDescriptionKey: "Could not allocate audio buffer"])
            }
            buffer.frameLength = AVAudioFrameCount(frames)
            let status = CMSampleBufferCopyPCMDataIntoAudioBufferList(sample, at: 0,
                                       frameCount: Int32(frames), into: buffer.mutableAudioBufferList)
            guard status == noErr else {
                throw NSError(domain: NSOSStatusErrorDomain, code: Int(status))
            }
            do { try file?.write(from: buffer) }
            catch { throw NSError(domain: "TervyMeeting", code: 11, userInfo: [NSLocalizedDescriptionKey: "Write audio file: \(error)"]) }
        } catch {
            failure = error
        }
    }

    func finish() throws {
        if let failure { throw failure }
        file = nil
    }
}

final class CaptureDelegate: NSObject, SCStreamOutput, SCStreamDelegate {
    let system: AudioWriter
    let microphone: AudioWriter
    var failure: ((Error) -> Void)?

    init(system: AudioWriter, microphone: AudioWriter) {
        self.system = system
        self.microphone = microphone
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sample: CMSampleBuffer, of type: SCStreamOutputType) {
        switch type {
        case .audio: system.append(sample)
        case .microphone: microphone.append(sample)
        default: break
        }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) { failure?(error) }
}

func reportCaptureFailure(_ stage: String, _ error: Error) {
    let nsError = error as NSError
    let bundleID = Bundle.main.bundleIdentifier ?? "unknown"
    let screenAccess = CGPreflightScreenCaptureAccess()
    let microphoneAccess = AVCaptureDevice.authorizationStatus(for: .audio).rawValue
    fputs("Capture diagnostics: stage=\(stage); bundle=\(bundleID); screenAccess=\(screenAccess); microphoneAccess=\(microphoneAccess); errorDomain=\(nsError.domain); errorCode=\(nsError.code)\n", stderr)
}

func authorizeMicrophone() async throws {
    let initial = AVCaptureDevice.authorizationStatus(for: .audio)
    var requested = false
    switch initial {
    case .authorized:
        return
    case .notDetermined:
        requested = true
        if await AVCaptureDevice.requestAccess(for: .audio) {
            return
        }
    case .denied, .restricted:
        break
    @unknown default:
        break
    }
    fputs("TERVY_MICROPHONE_PERMISSION_DENIED\n", stderr)
    fputs("Microphone diagnostics: bundle=\(Bundle.main.bundleIdentifier ?? "unknown"); initial=\(initial.rawValue); final=\(AVCaptureDevice.authorizationStatus(for: .audio).rawValue); requested=\(requested)\n", stderr)
    throw NSError(domain: "TervyMeeting", code: 12,
                  userInfo: [NSLocalizedDescriptionKey: "Microphone access was denied"])
}

func record(systemURL: URL, microphoneURL: URL) async throws {
    try await authorizeMicrophone()
    let content: SCShareableContent
    do {
        content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
    } catch {
        reportCaptureFailure("shareable-content", error)
        throw error
    }
    guard let display = content.displays.first else {
        throw NSError(domain: "TervyMeeting", code: 1, userInfo: [NSLocalizedDescriptionKey: "No display is available for system audio capture"])
    }
    let filter = SCContentFilter(display: display, excludingWindows: [])
    let configuration = SCStreamConfiguration()
    configuration.capturesAudio = true
    configuration.captureMicrophone = true
    configuration.excludesCurrentProcessAudio = true
    configuration.sampleRate = 48_000
    configuration.channelCount = 2
    let delegate = try CaptureDelegate(system: AudioWriter(url: systemURL), microphone: AudioWriter(url: microphoneURL))
    delegate.failure = { error in
        fputs("\(String(reflecting: error))\n", stderr)
        exit(1)
    }
    let stream = SCStream(filter: filter, configuration: configuration, delegate: delegate)
    let sampleQueue = DispatchQueue(label: "tervy.meeting-audio")
    try stream.addStreamOutput(delegate, type: .audio, sampleHandlerQueue: sampleQueue)
    try stream.addStreamOutput(delegate, type: .microphone, sampleHandlerQueue: sampleQueue)
    do {
        try await stream.startCapture()
    } catch {
        reportCaptureFailure("start-capture", error)
        throw error
    }
    print("READY")
    fflush(stdout)
    _ = await withCheckedContinuation { continuation in
        DispatchQueue.global(qos: .userInitiated).async {
            _ = readLine()
            continuation.resume()
        }
    }
    try await stream.stopCapture()
    sampleQueue.sync {}
    try delegate.system.finish()
    try delegate.microphone.finish()
    print("DONE")
    fflush(stdout)
}

func transcribe(_ url: URL) async throws {
    let supportedLocale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale.current)
    guard SpeechTranscriber.isAvailable, let locale = supportedLocale else {
        throw NSError(domain: "TervyMeeting", code: 3, userInfo: [NSLocalizedDescriptionKey: "On-device transcription is unavailable for \(Locale.current.localizedString(forIdentifier: Locale.current.identifier) ?? Locale.current.identifier) on this Mac"])
    }
    let transcriber = SpeechTranscriber(locale: locale, transcriptionOptions: [], reportingOptions: [], attributeOptions: [.audioTimeRange])
    if let installation = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
        try await installation.downloadAndInstall()
    }
    let audioFile = try AVAudioFile(forReading: url)
    let analyzer = SpeechAnalyzer(modules: [transcriber])
    async let segments = transcriber.results.reduce([[String: Any]]()) { rows, result in
        var rows = rows
        rows.append(["start": result.range.start.seconds, "text": String(result.text.characters)])
        return rows
    }
    if let lastSample = try await analyzer.analyzeSequence(from: audioFile) {
        try await analyzer.finalizeAndFinish(through: lastSample)
    } else {
        await analyzer.cancelAndFinishNow()
    }
    let data = try JSONSerialization.data(withJSONObject: try await segments)
    print(String(decoding: data, as: UTF8.self))
}

@main struct MeetingAudio {
    static func main() async {
        let arguments = CommandLine.arguments
        guard arguments.count >= 2 else {
            fputs("Usage: meeting-audio authorize | record SYSTEM MIC | transcribe FILE\n", stderr)
            exit(2)
        }
        do {
            switch arguments[1] {
            case "authorize" where arguments.count == 2:
                try await authorizeMicrophone()
            case "record" where arguments.count == 4:
                try await record(systemURL: URL(fileURLWithPath: arguments[2]), microphoneURL: URL(fileURLWithPath: arguments[3]))
            case "transcribe" where arguments.count == 3:
                try await transcribe(URL(fileURLWithPath: arguments[2]))
            default:
                throw NSError(domain: "TervyMeeting", code: 4, userInfo: [NSLocalizedDescriptionKey: "Unknown command"])
            }
        } catch {
            fputs("\(String(reflecting: error))\n", stderr)
            exit(1)
        }
    }
}
