import SwiftUI

struct SidebarView: View {
    let model: WorkspaceModel
    @State private var search = ""
    @State private var selection: TableRef.ID?
    @State private var pendingAction: DestructiveAction?

    struct DestructiveAction {
        var table: TableRef
        var drop: Bool
    }

    private var filtered: [TableRef] {
        let query = search.trimmingCharacters(in: .whitespaces).lowercased()
        guard !query.isEmpty else { return model.tables }
        return model.tables.filter { $0.name.lowercased().contains(query) }
    }

    var body: some View {
        VStack(spacing: 0) {
            if model.kind == .postgres, !model.schemas.isEmpty {
                schemaPicker
                Divider()
            }
            list
            Divider()
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.secondary)
                TextField("Search tables", text: $search)
                    .textFieldStyle(.plain)
                if !search.isEmpty {
                    Button {
                        search = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
        }
        .onChange(of: selection) { _, id in
            if let table = model.tables.first(where: { $0.id == id }) { model.open(table) }
        }
        .onChange(of: model.selectedTabID) { _, _ in
            selection = model.selectedTab?.kind.table?.id
        }
        .alert(
            alertTitle,
            isPresented: Binding(get: { pendingAction != nil }, set: { if !$0 { pendingAction = nil } })
        ) {
            Button(pendingAction?.drop == true ? "Drop" : "Truncate", role: .destructive) {
                guard let action = pendingAction else { return }
                Task { await model.dropOrTruncate(action.table, drop: action.drop) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This cannot be undone.")
        }
    }

    private var alertTitle: String {
        guard let action = pendingAction else { return "" }
        return "\(action.drop ? "Drop" : "Truncate") “\(action.table.name)”?"
    }

    private var schemaPicker: some View {
        Picker("Schema", selection: Binding(
            get: { model.currentSchema ?? "" },
            set: { name in Task { await model.selectSchema(name) } }
        )) {
            ForEach(model.schemas, id: \.self) { Text($0).tag($0) }
        }
        .labelsHidden()
        .padding(8)
    }

    private var list: some View {
        let tables = filtered.filter { $0.kind == .table }
        let views = filtered.filter { $0.kind == .view }
        return List(selection: $selection) {
            if let error = model.sidebarError {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.red)
                    .textSelection(.enabled)
            }
            if model.kind == .mysql && model.currentDatabase == nil {
                Text("Choose a database from the toolbar.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            if !tables.isEmpty {
                Section("Tables (\(tables.count))") {
                    ForEach(tables) { row($0) }
                }
            }
            if !views.isEmpty {
                Section("Views (\(views.count))") {
                    ForEach(views) { row($0) }
                }
            }
        }
        .listStyle(.sidebar)
        .overlay {
            if model.isLoadingTables && model.tables.isEmpty {
                ProgressView().controlSize(.small)
            }
        }
        .contextMenu(forSelectionType: TableRef.ID.self) { ids in
            if let id = ids.first, let table = model.tables.first(where: { $0.id == id }) {
                menu(for: table)
            }
        }
    }

    private func row(_ table: TableRef) -> some View {
        Label {
            Text(table.name).lineLimit(1)
        } icon: {
            Image(systemName: table.kind == .view ? "eye" : "tablecells")
                .foregroundStyle(table.kind == .view ? Color.purple : Color.accentColor)
        }
        .tag(table.id)
    }

    @ViewBuilder
    private func menu(for table: TableRef) -> some View {
        Button("Open in New Tab") { model.open(table, newTab: true) }
        Button("Query in New Tab") {
            model.newQueryTab(sql: "SELECT * FROM \(model.kind.qualifiedName(table)) LIMIT 100;")
        }
        Button("Copy Name") {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(table.name, forType: .string)
        }
        Divider()
        if table.kind == .table {
            Button("Truncate…", role: .destructive) { pendingAction = DestructiveAction(table: table, drop: false) }
        }
        Button("Drop…", role: .destructive) { pendingAction = DestructiveAction(table: table, drop: true) }
    }
}
