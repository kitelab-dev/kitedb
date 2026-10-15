import Foundation
import Observation

/// State of one open connection window: the session, the sidebar contents
/// and the open tabs.
@MainActor
@Observable
final class WorkspaceModel {
    enum Status: Equatable {
        case connecting
        case connected
        case failed(String)
    }

    let config: ConnectionConfig
    private(set) var session: DBSession?
    private(set) var status: Status = .connecting
    private(set) var serverVersion = ""

    private(set) var databases: [String] = []
    private(set) var currentDatabase: String?
    private(set) var schemas: [String] = []
    private(set) var currentSchema: String?
    private(set) var tables: [TableRef] = []
    private(set) var isLoadingTables = false
    var sidebarError: String?

    var tabs: [WorkspaceTab] = []
    var selectedTabID: UUID?
    private var queryCounter = 0

    init(config: ConnectionConfig) {
        self.config = config
    }

    var kind: DatabaseKind { config.kind }

    var selectedTab: WorkspaceTab? {
        tabs.first { $0.id == selectedTabID }
    }

    /// All table and column names, used by the SQL editor's completion.
    var completionWords: [String] {
        tables.map(\.name)
    }

    // MARK: - Connection

    func connect() async {
        status = .connecting
        let driver = DriverFactory.make(config: config, password: Keychain.password(for: config.id))
        let session = DBSession(driver: driver)
        do {
            let info = try await session.run { driver -> (String, [String], String?) in
                try driver.connect()
                let databases = (try? driver.databases()) ?? []
                return (driver.serverVersion, databases, driver.currentDatabase)
            }
            self.session = session
            serverVersion = info.0
            databases = info.1
            currentDatabase = info.2
            status = .connected
            await loadSchemasAndTables()
            if tabs.isEmpty { newQueryTab() }
        } catch {
            status = .failed(error.localizedDescription)
        }
    }

    func disconnect() {
        session?.close()
        session = nil
    }

    // MARK: - Sidebar

    func refresh() async {
        guard let session else { return }
        databases = (try? await session.run { try $0.databases() }) ?? databases
        await loadSchemasAndTables()
    }

    func selectDatabase(_ name: String) async {
        guard let session, name != currentDatabase else { return }
        do {
            try await session.run { try $0.useDatabase(name) }
            currentDatabase = name
            currentSchema = nil
            // Table tabs belong to the previous database.
            tabs.removeAll { $0.kind.isTable }
            if selectedTab == nil { selectedTabID = tabs.last?.id }
            await loadSchemasAndTables()
        } catch {
            sidebarError = error.localizedDescription
        }
    }

    func selectSchema(_ name: String) async {
        guard name != currentSchema else { return }
        currentSchema = name
        await loadTables()
    }

    private func loadSchemasAndTables() async {
        guard let session else { return }
        if kind == .postgres {
            schemas = (try? await session.run { try $0.schemas() }) ?? []
            if currentSchema == nil || !schemas.contains(currentSchema!) {
                currentSchema = schemas.first
            }
        }
        await loadTables()
    }

    private func loadTables() async {
        guard let session else { return }
        isLoadingTables = true
        defer { isLoadingTables = false }
        let schema = currentSchema
        do {
            tables = try await session.run { try $0.tables(schema: schema) }
            sidebarError = nil
        } catch {
            tables = []
            sidebarError = error.localizedDescription
        }
    }

    func dropOrTruncate(_ table: TableRef, drop: Bool) async {
        guard let session else { return }
        let name = kind.qualifiedName(table)
        let sql: String
        if drop {
            sql = "DROP \(table.kind == .view ? "VIEW" : "TABLE") \(name)"
        } else {
            sql = "TRUNCATE TABLE \(name)"
        }
        do {
            _ = try await session.run { try $0.execute(sql) }
            if drop {
                closeTabs { $0.kind.table == table }
            } else {
                for case let .table(model) in tabs.map(\.kind) where model.table == table {
                    await model.reload()
                }
            }
            await loadTables()
        } catch {
            sidebarError = error.localizedDescription
        }
    }

    // MARK: - Tabs

    /// Opens a table. Reuses an existing tab for it, otherwise replaces the
    /// current table tab when it has no pending edits (like a preview tab).
    func open(_ table: TableRef, newTab: Bool = false) {
        guard let session else { return }
        if let existing = tabs.first(where: { $0.kind.table == table }) {
            selectedTabID = existing.id
            return
        }
        let model = TableTabModel(table: table, session: session, kind: kind)
        let tab = WorkspaceTab(kind: .table(model))
        if !newTab, let current = selectedTab, case let .table(currentModel) = current.kind,
           !currentModel.hasChanges, let index = tabs.firstIndex(where: { $0.id == current.id }) {
            tabs[index] = tab
        } else {
            tabs.append(tab)
        }
        selectedTabID = tab.id
        Task { await model.load() }
    }

    func newQueryTab(sql: String = "") {
        guard let session else { return }
        queryCounter += 1
        let model = QueryTabModel(title: "Query \(queryCounter)", session: session)
        model.sql = sql
        let tab = WorkspaceTab(kind: .query(model))
        tabs.append(tab)
        selectedTabID = tab.id
    }

    func close(_ tab: WorkspaceTab) {
        closeTabs { $0.id == tab.id }
    }

    private func closeTabs(where predicate: (WorkspaceTab) -> Bool) {
        let selectedIndex = tabs.firstIndex { $0.id == selectedTabID }
        tabs.removeAll(where: predicate)
        if selectedTab == nil {
            let index = min(selectedIndex ?? 0, tabs.count - 1)
            selectedTabID = index >= 0 ? tabs[index].id : nil
        }
    }

    func selectTab(offset: Int) {
        guard !tabs.isEmpty else { return }
        let index = tabs.firstIndex { $0.id == selectedTabID } ?? 0
        selectedTabID = tabs[(index + offset + tabs.count) % tabs.count].id
    }
}

struct WorkspaceTab: Identifiable {
    enum Kind {
        case table(TableTabModel)
        case query(QueryTabModel)

        var isTable: Bool { table != nil }

        var table: TableRef? {
            if case let .table(model) = self { return model.table }
            return nil
        }
    }

    let id = UUID()
    let kind: Kind

    @MainActor var title: String {
        switch kind {
        case let .table(model): model.table.name
        case let .query(model): model.title
        }
    }

    var icon: String {
        switch kind {
        case let .table(model): model.table.kind == .view ? "eye" : "tablecells"
        case .query: "terminal"
        }
    }
}
