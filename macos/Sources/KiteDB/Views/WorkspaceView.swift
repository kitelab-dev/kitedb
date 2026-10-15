import SwiftUI

/// Resolves the window's connection id and owns its workspace model.
struct WorkspaceRoot: View {
    let connectionID: UUID?
    @Environment(\.openWindow) private var openWindow
    @State private var model: WorkspaceModel?

    var body: some View {
        Group {
            if let model {
                WorkspaceView(model: model)
            } else if connectionID.flatMap(ConnectionStore.shared.connection(id:)) == nil {
                ContentUnavailableView("Connection not found", systemImage: "questionmark.circle")
            } else {
                ProgressView()
            }
        }
        .frame(minWidth: 900, minHeight: 520)
        .task {
            guard model == nil, let connectionID, let config = ConnectionStore.shared.connection(id: connectionID) else { return }
            let model = WorkspaceModel(config: config)
            self.model = model
            WorkspaceModel.openCount += 1
            await model.connect()
        }
        .onDisappear {
            guard let model else { return }
            model.disconnect()
            WorkspaceModel.openCount -= 1
            if WorkspaceModel.openCount == 0 { openWindow(id: WindowID.welcome) }
        }
    }
}

extension WorkspaceModel {
    static var openCount = 0
}

struct WorkspaceView: View {
    @Bindable var model: WorkspaceModel

    var body: some View {
        NavigationSplitView {
            SidebarView(model: model)
                .navigationSplitViewColumnWidth(min: 200, ideal: 250, max: 420)
        } detail: {
            detail
        }
        .navigationTitle(model.config.displayName)
        .navigationSubtitle(model.currentDatabase ?? "")
        .toolbar { toolbar }
        .focusedSceneValue(\.workspace, model)
    }

    @ViewBuilder
    private var detail: some View {
        switch model.status {
        case .connecting:
            ProgressView("Connecting to \(model.config.host)…")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case let .failed(message):
            ContentUnavailableView {
                Label("Connection Failed", systemImage: "bolt.horizontal.circle")
            } description: {
                Text(message).textSelection(.enabled)
            } actions: {
                Button("Try Again") { Task { await model.connect() } }
            }
        case .connected:
            VStack(spacing: 0) {
                TabBar(model: model)
                Divider()
                if let tab = model.selectedTab {
                    tabContent(tab).id(tab.id)
                } else {
                    ContentUnavailableView {
                        Label("No Open Tabs", systemImage: "tablecells")
                    } description: {
                        Text("Pick a table from the sidebar or open a new query with ⌘T.")
                    } actions: {
                        Button("New Query") { model.newQueryTab() }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func tabContent(_ tab: WorkspaceTab) -> some View {
        switch tab.kind {
        case let .table(tableModel):
            TableTabView(model: tableModel)
        case let .query(queryModel):
            QueryTabView(model: queryModel, completionWords: model.completionWords)
        }
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .navigation) {
            ConnectionBadge(model: model)
        }
        ToolbarItemGroup(placement: .primaryAction) {
            if model.status == .connected {
                Menu {
                    ForEach(model.databases, id: \.self) { name in
                        Button {
                            Task { await model.selectDatabase(name) }
                        } label: {
                            if name == model.currentDatabase {
                                Label(name, systemImage: "checkmark")
                            } else {
                                Text(name)
                            }
                        }
                    }
                } label: {
                    Label(model.currentDatabase ?? "Database", systemImage: "cylinder")
                        .labelStyle(.titleAndIcon)
                }
                .help("Switch database")

                Button {
                    Task { await model.refresh() }
                } label: {
                    Label("Refresh", systemImage: "arrow.clockwise")
                }
                .help("Refresh (⌘R)")

                Button {
                    model.newQueryTab()
                } label: {
                    Label("New Query", systemImage: "plus.rectangle.on.rectangle")
                }
                .help("New query tab (⌘T)")
            }
        }
    }
}

struct ConnectionBadge: View {
    let model: WorkspaceModel

    var body: some View {
        HStack(spacing: 8) {
            KindBadge(kind: model.kind, size: 20)
            VStack(alignment: .leading, spacing: 0) {
                Text(model.config.displayName)
                    .font(.system(size: 12, weight: .semibold))
                Text(statusText)
                    .font(.system(size: 10))
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 3)
        .background {
            RoundedRectangle(cornerRadius: 7, style: .continuous)
                .fill((model.config.color.color ?? .secondary).opacity(0.18))
        }
    }

    private var statusText: String {
        switch model.status {
        case .connecting: "Connecting…"
        case .failed: "Disconnected"
        case .connected:
            "\(model.kind.displayName) \(model.serverVersion.split(separator: " ").first ?? "")"
        }
    }
}

struct TabBar: View {
    let model: WorkspaceModel

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 0) {
                ForEach(model.tabs) { tab in
                    TabItemView(
                        title: tab.title,
                        icon: tab.icon,
                        isSelected: tab.id == model.selectedTabID,
                        onSelect: { model.selectedTabID = tab.id },
                        onClose: { model.close(tab) }
                    )
                    Divider()
                }
                Button {
                    model.newQueryTab()
                } label: {
                    Image(systemName: "plus")
                        .frame(width: 28, height: 28)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .help("New query tab (⌘T)")
            }
        }
        .frame(height: 30)
        .background(.bar)
    }
}

struct TabItemView: View {
    let title: String
    let icon: String
    let isSelected: Bool
    let onSelect: () -> Void
    let onClose: () -> Void
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: icon)
                .font(.system(size: 11))
                .foregroundStyle(isSelected ? Color.accentColor : .secondary)
            Text(title)
                .font(.system(size: 12))
                .lineLimit(1)
            Spacer(minLength: 4)
            Button(action: onClose) {
                Image(systemName: "xmark")
                    .font(.system(size: 9, weight: .bold))
                    .frame(width: 16, height: 16)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .opacity(hovering || isSelected ? 1 : 0)
        }
        .padding(.horizontal, 10)
        .frame(minWidth: 110, maxWidth: 200, minHeight: 30, maxHeight: 30)
        .background(isSelected ? Color(nsColor: .controlBackgroundColor) : Color.clear)
        .overlay(alignment: .bottom) {
            if isSelected { Rectangle().fill(Color.accentColor).frame(height: 2) }
        }
        .contentShape(Rectangle())
        .onTapGesture(perform: onSelect)
        .onHover { hovering = $0 }
        .contextMenu {
            Button("Close Tab", action: onClose)
        }
    }
}
