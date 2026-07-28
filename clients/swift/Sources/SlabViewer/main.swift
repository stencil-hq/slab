#if canImport(AppKit)
import AppKit
import Slab
import SlabAppKit

@MainActor
private final class ViewerDelegate: NSObject, NSApplicationDelegate {
    private var window: NSWindow?
    private var runtime: SlabRuntime?
    private var session: SlabSession?

    func applicationDidFinishLaunching(_ notification: Notification) {
        guard let path = sourcePath() else {
            NSApplication.shared.terminate(nil)
            return
        }

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 900, height: 700),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = URL(fileURLWithPath: path).lastPathComponent
        window.center()
        window.contentView = loadingView()
        window.makeKeyAndOrderFront(nil)
        self.window = window
        NSApplication.shared.activate(ignoringOtherApps: true)

        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                let source = try String(contentsOfFile: path, encoding: .utf8)
                let runtime = try await Task.detached(priority: .userInitiated) {
                    try SlabRuntime()
                }.value
                let session = try await runtime.makeSession()
                let slabView = try SlabView(session: session)
                slabView.onSignals = { signals in
                    for signal in signals {
                        print("signal \(signal.name) text=\(signal.text.debugDescription) item=\(signal.item.debugDescription)")
                    }
                }
                slabView.onDiagnostics = { notes in
                    for note in notes {
                        FileHandle.standardError.write(Data("slab: \(note)\n".utf8))
                    }
                }
                slabView.onError = { [weak self] error in
                    self?.present(error)
                }
                self.runtime = runtime
                self.session = session
                window.contentView = slabView
                slabView.load(source: source, name: path)
                window.makeFirstResponder(slabView)
            } catch {
                present(error)
            }
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        guard let session else { return }
        Task { await session.close() }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    private func sourcePath() -> String? {
        let arguments = Array(CommandLine.arguments.dropFirst())
        guard arguments.count == 1, !["-h", "--help"].contains(arguments[0]) else {
            FileHandle.standardError.write(Data("usage: slab-swift FILE.slab\n".utf8))
            return nil
        }
        return URL(fileURLWithPath: arguments[0]).standardizedFileURL.path
    }

    private func loadingView() -> NSView {
        let label = NSTextField(labelWithString: "Loading Slab runtime…")
        label.alignment = .center
        label.textColor = .secondaryLabelColor
        label.translatesAutoresizingMaskIntoConstraints = false
        let view = NSView()
        view.addSubview(label)
        NSLayoutConstraint.activate([
            label.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            label.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])
        return view
    }

    private func present(_ error: any Error) {
        FileHandle.standardError.write(Data("slab: \(error.localizedDescription)\n".utf8))
        let alert = NSAlert(error: error)
        if let window {
            alert.beginSheetModal(for: window)
        } else {
            alert.runModal()
        }
    }
}

let application = NSApplication.shared
private let delegate = ViewerDelegate()
application.setActivationPolicy(.regular)
application.delegate = delegate
application.run()
#else
import Foundation

FileHandle.standardError.write(Data("slab-swift requires macOS AppKit\n".utf8))
#endif
