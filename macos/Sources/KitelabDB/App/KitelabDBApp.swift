import CMySQL
import SwiftUI

@main
struct KitelabDBApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        Window("Kitelab DB", id: WindowID.welcome) {
            WelcomeView()
                .frame(minWidth: 700, minHeight: 420)
        }
        .defaultSize(width: 820, height: 500)
        .windowResizability(.contentMinSize)
        .commands {
            AppCommands()
            WorkspaceCommands()
        }

        WindowGroup("Connection", id: WindowID.workspace, for: UUID.self) { $connectionID in
            WorkspaceRoot(connectionID: connectionID)
        }
        .defaultSize(width: 1280, height: 800)
    }
}

enum WindowID {
    static let welcome = "welcome"
    static let workspace = "workspace"
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationWillFinishLaunching(_ notification: Notification) {
        // libmysqlclient's global init is not thread-safe; do it up front.
        mysql_server_init(0, nil, nil)
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
    }
}

struct AppCommands: Commands {
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("Connections…") {
                openWindow(id: WindowID.welcome)
            }
            .keyboardShortcut("k", modifiers: [.command, .shift])
        }
    }
}

struct WorkspaceFocusKey: FocusedValueKey {
    typealias Value = WorkspaceModel
}

extension FocusedValues {
    var workspace: WorkspaceModel? {
        get { self[WorkspaceFocusKey.self] }
        set { self[WorkspaceFocusKey.self] = newValue }
    }
}

struct WorkspaceCommands: Commands {
    @FocusedValue(\.workspace) private var workspace

    private var queryTab: QueryTabModel? {
        if case let .query(model) = workspace?.selectedTab?.kind { return model }
        return nil
    }

    private var tableTab: TableTabModel? {
        if case let .table(model) = workspace?.selectedTab?.kind { return model }
        return nil
    }

    var body: some Commands {
        CommandMenu("Query") {
            Button("New Query Tab") { workspace?.newQueryTab() }
                .keyboardShortcut("t")
                .disabled(workspace?.status != .connected)
            Button("Run Query") {
                if let queryTab { Task { await queryTab.run() } }
            }
            .keyboardShortcut(.return)
            .disabled(queryTab == nil)
            Button("Cancel Query") { queryTab?.cancel() }
                .keyboardShortcut(".")
                .disabled(queryTab?.isRunning != true)

            Divider()

            Button("Save Changes") {
                if let tableTab { Task { await tableTab.saveChanges() } }
            }
            .keyboardShortcut("s")
            .disabled(tableTab?.hasChanges != true)
            Button("Discard Changes") { tableTab?.discardChanges() }
                .disabled(tableTab?.hasChanges != true)
            Button("Add Row") { tableTab?.addRow() }
                .keyboardShortcut("i")
                .disabled(tableTab?.canEdit != true)

            Divider()

            Button("Refresh") {
                guard let workspace else { return }
                Task {
                    if let tableTab, !tableTab.hasChanges { await tableTab.reload() }
                    await workspace.refresh()
                }
            }
            .keyboardShortcut("r")
            .disabled(workspace?.status != .connected)

            Divider()

            Button("Next Tab") { workspace?.selectTab(offset: 1) }
                .keyboardShortcut("]", modifiers: [.command, .shift])
            Button("Previous Tab") { workspace?.selectTab(offset: -1) }
                .keyboardShortcut("[", modifiers: [.command, .shift])
            Button("Close Tab") {
                if let tab = workspace?.selectedTab { workspace?.close(tab) }
            }
            .keyboardShortcut("w", modifiers: [.command, .option])
            .disabled(workspace?.selectedTab == nil)
        }
    }
}
