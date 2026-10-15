import SwiftUI

struct WelcomeView: View {
    @Environment(\.openWindow) private var openWindow
    @Environment(\.dismissWindow) private var dismissWindow
    @State private var store = ConnectionStore.shared
    @State private var search = ""
    @State private var selection: UUID?
    @State private var editing: EditingConnection?
    @State private var pendingDelete: ConnectionConfig?

    struct EditingConnection: Identifiable {
        var config: ConnectionConfig
        var isNew: Bool
        var id: UUID { config.id }
    }

    private var filtered: [ConnectionConfig] {
        let query = search.trimmingCharacters(in: .whitespaces).lowercased()
        guard !query.isEmpty else { return store.connections }
        return store.connections.filter {
            $0.displayName.lowercased().contains(query) || $0.summary.lowercased().contains(query)
        }
    }

    var body: some View {
        HStack(spacing: 0) {
            sidePanel
            Divider()
            connectionList
        }
        .sheet(item: $editing) { item in
            ConnectionFormView(config: item.config, isNew: item.isNew) { config, password, connectNow in
                store.save(config, password: password)
                selection = config.id
                editing = nil
                if connectNow { connect(config.id) }
            }
        }
        .alert(
            "Delete “\(pendingDelete?.displayName ?? "")”?",
            isPresented: Binding(get: { pendingDelete != nil }, set: { if !$0 { pendingDelete = nil } })
        ) {
            Button("Delete", role: .destructive) {
                if let pendingDelete { store.delete(pendingDelete) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The saved connection and its password will be removed.")
        }
    }

    private var sidePanel: some View {
        VStack(spacing: 14) {
            Spacer().frame(height: 24)
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 96, height: 96)
            VStack(spacing: 4) {
                Text("KiteDB")
                    .font(.title2.weight(.semibold))
                Text("MySQL & PostgreSQL client")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button {
                editing = EditingConnection(config: ConnectionConfig(), isNew: true)
            } label: {
                Label("New Connection…", systemImage: "plus")
                    .frame(maxWidth: .infinity)
            }
            .controlSize(.large)
            .keyboardShortcut("n")
            Link(destination: AppLinks.donate) {
                Label("Support KiteDB", systemImage: "heart.fill")
                    .font(.callout)
            }
            .tint(.pink)
            .help("Donate via Byl")
            Text("Version \(appVersion)")
                .font(.caption)
                .foregroundStyle(.tertiary)
                .padding(.bottom, 16)
        }
        .padding(.horizontal, 24)
        .frame(width: 250)
        .background(.background.secondary)
    }

    private var appVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
    }

    private var connectionList: some View {
        VStack(spacing: 0) {
            HStack {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Search connections", text: $search)
                    .textFieldStyle(.plain)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            Divider()

            if store.connections.isEmpty {
                ContentUnavailableView {
                    Label("No Connections", systemImage: "cylinder.split.1x2")
                } description: {
                    Text("Create a connection to a MySQL or PostgreSQL server.")
                } actions: {
                    Button("New Connection…") {
                        editing = EditingConnection(config: ConnectionConfig(), isNew: true)
                    }
                }
            } else {
                List(selection: $selection) {
                    ForEach(filtered) { config in
                        ConnectionRow(config: config).tag(config.id)
                    }
                    .onMove { store.move(from: $0, to: $1) }
                }
                .listStyle(.inset)
                .contextMenu(forSelectionType: UUID.self) { ids in
                    if let id = ids.first, let config = store.connection(id: id) {
                        Button("Connect") { connect(id) }
                        Divider()
                        Button("Edit…") { editing = EditingConnection(config: config, isNew: false) }
                        Button("Duplicate") { store.duplicate(config) }
                        Divider()
                        Button("Delete…", role: .destructive) { pendingDelete = config }
                    }
                } primaryAction: { ids in
                    if let id = ids.first { connect(id) }
                }
                .onKeyPress(.return) {
                    guard let selection else { return .ignored }
                    connect(selection)
                    return .handled
                }
            }
        }
    }

    private func connect(_ id: UUID) {
        openWindow(id: WindowID.workspace, value: id)
        dismissWindow(id: WindowID.welcome)
    }
}

struct ConnectionRow: View {
    let config: ConnectionConfig

    var body: some View {
        HStack(spacing: 10) {
            KindBadge(kind: config.kind)
            VStack(alignment: .leading, spacing: 2) {
                Text(config.displayName)
                    .fontWeight(.medium)
                Text(config.summary)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer()
            if let color = config.color.color {
                Circle().fill(color).frame(width: 9, height: 9)
            }
        }
        .padding(.vertical, 4)
    }
}

struct KindBadge: View {
    let kind: DatabaseKind
    var size: CGFloat = 32

    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.25, style: .continuous)
            .fill(kind.badgeColor.gradient)
            .frame(width: size, height: size)
            .overlay {
                Text(kind == .mysql ? "My" : "Pg")
                    .font(.system(size: size * 0.4, weight: .bold, design: .rounded))
                    .foregroundStyle(.white)
            }
    }
}
