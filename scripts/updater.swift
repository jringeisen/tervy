import AppKit
import Sparkle

public typealias EventCallback = @convention(c) (Int32, UnsafePointer<CChar>?) -> Void

@MainActor
private final class TervyUpdater: NSObject, SPUUpdaterDelegate {
    let callback: EventCallback
    var controller: SPUStandardUpdaterController!
    var resumeInstall: (() -> Void)?

    init(callback: @escaping EventCallback) {
        self.callback = callback
        super.init()
        controller = SPUStandardUpdaterController(
            startingUpdater: false,
            updaterDelegate: self,
            userDriverDelegate: nil
        )
        controller.startUpdater()
        if controller.updater.automaticallyChecksForUpdates {
            controller.updater.checkForUpdateInformation()
        }
    }

    private func emit(_ kind: Int32, _ value: String? = nil) {
        if let value {
            value.withCString { callback(kind, $0) }
        } else {
            callback(kind, nil)
        }
    }

    func updater(_ updater: SPUUpdater, didFindValidUpdate item: SUAppcastItem) {
        emit(1, item.displayVersionString)
    }

    func updaterDidNotFindUpdate(_ updater: SPUUpdater) {
        emit(0)
    }

    func updater(_ updater: SPUUpdater, didAbortWithError error: Error) {
        emit(2, error.localizedDescription)
    }

    func updater(
        _ updater: SPUUpdater,
        shouldPostponeRelaunchForUpdate item: SUAppcastItem,
        untilInvokingBlock installHandler: @escaping () -> Void
    ) -> Bool {
        resumeInstall = installHandler
        emit(3, item.displayVersionString)
        return true
    }
}

@MainActor private var sharedUpdater: TervyUpdater?

@_cdecl("tervy_updater_start")
public func tervyUpdaterStart(_ callback: EventCallback?) -> Int32 {
    MainActor.assumeIsolated {
        guard let callback,
              let feed = Bundle.main.object(forInfoDictionaryKey: "SUFeedURL") as? String,
              feed.hasPrefix("https://"),
              Bundle.main.object(forInfoDictionaryKey: "SUPublicEDKey") is String
        else { return 0 }
        if sharedUpdater == nil {
            sharedUpdater = TervyUpdater(callback: callback)
        }
        return 1
    }
}

@_cdecl("tervy_updater_check")
public func tervyUpdaterCheck() {
    MainActor.assumeIsolated {
        sharedUpdater?.controller.updater.checkForUpdates()
    }
}

@_cdecl("tervy_updater_resume_install")
public func tervyUpdaterResumeInstall() {
    MainActor.assumeIsolated {
        let handler = sharedUpdater?.resumeInstall
        sharedUpdater?.resumeInstall = nil
        handler?()
    }
}
