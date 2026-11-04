// Platform URL-event adapter. Rust validates and routes every forwarded address.
import AppKit

@MainActor
final class LinkLauncher: NSObject, NSApplicationDelegate {
    private var children: [Int32: Process] = [:]
    private var quitting = false
    private var receivedURL = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        DispatchQueue.main.async { [weak self] in
            guard let self, !self.receivedURL else { return }
            self.launch(nil)
        }
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        receivedURL = true
        for url in urls { launch(url) }
    }

    func application(_ application: NSApplication, continue activity: NSUserActivity,
                     restorationHandler: @escaping ([NSUserActivityRestoring]) -> Void) -> Bool {
        guard activity.activityType == NSUserActivityTypeBrowsingWeb,
              let url = activity.webpageURL else { return false }
        receivedURL = true
        launch(url)
        return true
    }

    func applicationShouldHandleReopen(_ application: NSApplication, hasVisibleWindows: Bool) -> Bool {
        launch(nil)
        return false
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        quitting = true
        for process in children.values where process.isRunning { process.terminate() }
        return .terminateNow
    }

    private func launch(_ url: URL?) {
        if let url, url.absoluteString.utf8.count > 16 * 1024 {
            showError()
            if children.isEmpty { NSApplication.shared.terminate(nil) }
            return
        }
        let process = Process()
        let server = Bundle.main.object(forInfoDictionaryKey: "AcheflowRecipient") as? String == "server"
        process.executableURL = executable(server: server)
        let input = Pipe()
        if url != nil {
            process.arguments = ["--forward-stdin"]
            process.standardInput = input
        } else {
            process.arguments = server ? ["--native-links"] : ["--show"]
        }
        // Never put the URL in process arguments, diagnostic output or disk files.
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        process.terminationHandler = { [weak self] process in
            let pid = process.processIdentifier
            let failed = process.terminationStatus != 0
            DispatchQueue.main.async {
                guard let self, !self.quitting else { return }
                self.finished(pid: pid, failed: failed)
            }
        }
        do {
            process.currentDirectoryURL = try workingDirectory(server: server)
            try process.run()
            children[process.processIdentifier] = process
            if let url {
                try input.fileHandleForWriting.write(contentsOf: Data(url.absoluteString.utf8))
                try input.fileHandleForWriting.close()
            }
        } catch {
            process.terminationHandler = nil
            children.removeValue(forKey: process.processIdentifier)
            if process.isRunning { process.terminate() }
            showError()
            if children.isEmpty { NSApplication.shared.terminate(nil) }
        }
    }

    private func executable(server: Bool) -> URL {
        if server {
            return Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/acheflow-server")
        }
        let contents = Bundle.main.bundleURL.deletingLastPathComponent().deletingLastPathComponent()
        return contents.appendingPathComponent("MacOS/acheflow-ui")
    }

    private func workingDirectory(server: Bool) throws -> URL {
        // Finder launches have no stable caller working directory.
        let support = try FileManager.default.url(for: .applicationSupportDirectory,
            in: .userDomainMask, appropriateFor: nil, create: true)
        let directory = support.appendingPathComponent(server ? "Acheflow/Server" : "Acheflow/Client", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }

    private func finished(pid: Int32, failed: Bool) {
        children.removeValue(forKey: pid)
        if failed { showError() }
        if children.isEmpty { NSApplication.shared.terminate(nil) }
    }

    private func showError() {
        let alert = NSAlert()
        alert.messageText = "Acheflow could not accept this request"
        alert.informativeText = "The application or extension may be unavailable, or the link may be invalid or expired. Check the extension's state and request a new link."
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }
}

@main
struct AcheflowLinkMain {
    @MainActor
    static func main() {
        let application = NSApplication.shared
        let delegate = LinkLauncher()
        application.delegate = delegate
        application.setActivationPolicy(.accessory)
        application.run()
    }
}
