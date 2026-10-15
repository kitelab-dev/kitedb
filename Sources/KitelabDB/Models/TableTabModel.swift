import Foundation
import Observation

struct CellKey: Hashable {
    var row: Int
    var column: Int
}

enum FilterOperator: String, CaseIterable, Identifiable {
    case equals = "="
    case notEquals = "!="
    case greater = ">"
    case greaterOrEqual = ">="
    case less = "<"
    case lessOrEqual = "<="
    case contains = "contains"
    case notContains = "not contains"
    case like = "LIKE"
    case `in` = "IN"
    case isNull = "IS NULL"
    case isNotNull = "IS NOT NULL"

    var id: String { rawValue }

    var needsValue: Bool { self != .isNull && self != .isNotNull }
}

struct TableFilter: Equatable {
    /// Empty column means `value` is a raw SQL `WHERE` clause.
    var column = ""
    var op = FilterOperator.equals
    var value = ""

    var isRaw: Bool { column.isEmpty }

    var isEmpty: Bool {
        isRaw ? value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty : false
    }

    func sql(kind: DatabaseKind, quote: (String) -> String) -> String? {
        if isRaw {
            let clause = value.trimmingCharacters(in: .whitespacesAndNewlines)
            return clause.isEmpty ? nil : clause
        }
        let name = kind.quoteIdentifier(column)
        let textName = kind == .postgres ? "\(name)::text" : name
        let like = kind == .postgres ? "ILIKE" : "LIKE"
        switch op {
        case .isNull, .isNotNull:
            return "\(name) \(op.rawValue)"
        case .contains:
            return "\(textName) \(like) \(quote("%\(value)%"))"
        case .notContains:
            return "\(textName) NOT \(like) \(quote("%\(value)%"))"
        case .like:
            return "\(textName) \(like) \(quote(value))"
        case .in:
            let items = value.split(separator: ",")
                .map { quote($0.trimmingCharacters(in: .whitespaces)) }
            return items.isEmpty ? nil : "\(name) IN (\(items.joined(separator: ", ")))"
        default:
            return "\(name) \(op.rawValue) \(quote(value))"
        }
    }
}

/// A table opened in a tab: one page of its rows plus the edits staged on
/// them, and its structure.
@MainActor
@Observable
final class TableTabModel {
    enum Mode: String, CaseIterable, Identifiable {
        case data = "Data"
        case structure = "Structure"
        var id: String { rawValue }
    }

    let table: TableRef
    let session: DBSession
    let kind: DatabaseKind

    var mode = Mode.data
    let pageSize = 300

    private(set) var columns: [ColumnInfo] = []
    private(set) var indexes: [IndexInfo] = []
    private(set) var resultColumns: [String] = []
    private(set) var rows: [[String?]] = []
    private(set) var loadedRowCount = 0
    private(set) var page = 0
    private(set) var sortColumn: String?
    private(set) var sortAscending = true
    private(set) var appliedFilter = TableFilter()
    var filter = TableFilter()

    private(set) var isLoading = false
    private(set) var duration: TimeInterval = 0
    var error: String?

    private(set) var edits: [CellKey: String?] = [:]
    private(set) var deletedRows: Set<Int> = []
    /// Bumped whenever rows or edits change, so the grid knows to redraw.
    private(set) var dataVersion = 0
    var selectedRows = IndexSet()

    init(table: TableRef, session: DBSession, kind: DatabaseKind) {
        self.table = table
        self.session = session
        self.kind = kind
    }

    var primaryKey: [String] { columns.filter(\.isPrimaryKey).map(\.name) }
    var canEdit: Bool { table.kind == .table && !primaryKey.isEmpty }
    var insertedRowCount: Int { rows.count - loadedRowCount }
    var hasChanges: Bool { !edits.isEmpty || !deletedRows.isEmpty || insertedRowCount > 0 }
    var changeCount: Int {
        Set(edits.keys.map(\.row)).union(deletedRows).union(loadedRowCount..<rows.count).count
    }
    var hasNextPage: Bool { loadedRowCount == pageSize }
    var firstRowNumber: Int { page * pageSize + 1 }

    var editDisabledReason: String? {
        if table.kind == .view { return "Views are read-only" }
        if primaryKey.isEmpty && !columns.isEmpty { return "Editing needs a primary key" }
        return nil
    }

    // MARK: - Loading

    func load() async {
        isLoading = true
        let table = self.table
        do {
            let structure = try await session.run { driver in
                (try driver.columns(of: table), try driver.indexes(of: table))
            }
            columns = structure.0
            indexes = structure.1
        } catch {
            self.error = error.localizedDescription
        }
        await reload()
    }

    func reload() async {
        isLoading = true
        defer { isLoading = false }
        let name = kind.qualifiedName(table)
        let kind = self.kind
        let filter = appliedFilter
        let order = sortColumn.map { "\(kind.quoteIdentifier($0)) \(sortAscending ? "ASC" : "DESC")" }
        let limit = pageSize
        let offset = page * pageSize
        let started = Date()
        do {
            let result = try await session.run { driver in
                var sql = "SELECT * FROM \(name)"
                if let clause = filter.sql(kind: kind, quote: driver.quoteLiteral) {
                    sql += " WHERE \(clause)"
                }
                if let order { sql += " ORDER BY \(order)" }
                sql += " LIMIT \(limit) OFFSET \(offset)"
                return try driver.query(sql)
            }
            duration = Date().timeIntervalSince(started)
            resultColumns = result.columns
            rows = result.rows
            loadedRowCount = rows.count
            error = nil
        } catch {
            rows = []
            loadedRowCount = 0
            self.error = error.localizedDescription
        }
        resetEdits()
    }

    func loadStructure() async {
        let table = self.table
        if let structure = try? await session.run({ driver in
            (try driver.columns(of: table), try driver.indexes(of: table))
        }) {
            columns = structure.0
            indexes = structure.1
        }
    }

    func applyFilter() async {
        appliedFilter = filter
        page = 0
        await reload()
    }

    func clearFilter() async {
        filter = TableFilter(column: filter.column, op: filter.op)
        await applyFilter()
    }

    func sort(by columnIndex: Int) async {
        guard resultColumns.indices.contains(columnIndex) else { return }
        let name = resultColumns[columnIndex]
        if sortColumn == name {
            if sortAscending {
                sortAscending = false
            } else {
                sortColumn = nil
            }
        } else {
            sortColumn = name
            sortAscending = true
        }
        page = 0
        await reload()
    }

    func goToPage(_ page: Int) async {
        guard page >= 0 else { return }
        self.page = page
        await reload()
    }

    // MARK: - Editing

    func originalValue(row: Int, column: Int) -> String? {
        rows[row][column]
    }

    func value(row: Int, column: Int) -> String? {
        if let edited = edits[CellKey(row: row, column: column)] { return edited }
        return rows[row][column]
    }

    func isEdited(row: Int, column: Int) -> Bool {
        edits[CellKey(row: row, column: column)] != nil
    }

    func isInserted(row: Int) -> Bool { row >= loadedRowCount }

    func setValue(_ value: String?, row: Int, column: Int) {
        guard canEdit, rows.indices.contains(row), !deletedRows.contains(row) else { return }
        let key = CellKey(row: row, column: column)
        if !isInserted(row: row) && value == rows[row][column] {
            edits.removeValue(forKey: key)
        } else {
            edits[key] = .some(value)
        }
        dataVersion += 1
    }

    func addRow() {
        guard canEdit else { return }
        rows.append(Array(repeating: nil, count: resultColumns.count))
        selectedRows = IndexSet(integer: rows.count - 1)
        dataVersion += 1
    }

    func deleteRows(_ indexes: IndexSet) {
        guard canEdit else { return }
        // Inserted rows are simply dropped; their edits shift down.
        for row in indexes.reversed() where isInserted(row: row) {
            rows.remove(at: row)
            var shifted: [CellKey: String?] = [:]
            for (key, value) in edits where key.row != row {
                shifted[CellKey(row: key.row > row ? key.row - 1 : key.row, column: key.column)] = value
            }
            edits = shifted
        }
        for row in indexes where row < loadedRowCount {
            deletedRows.insert(row)
            edits = edits.filter { $0.key.row != row }
        }
        selectedRows = []
        dataVersion += 1
    }

    func discardChanges() {
        rows.removeLast(insertedRowCount)
        resetEdits()
    }

    private func resetEdits() {
        edits = [:]
        deletedRows = []
        selectedRows = []
        dataVersion += 1
    }

    /// Writes all staged edits, inserts and deletes in one transaction.
    func saveChanges() async {
        guard hasChanges else { return }
        let kind = self.kind
        let name = kind.qualifiedName(table)
        let columnNames = resultColumns
        let primaryKey = self.primaryKey
        let pkIndexes = primaryKey.compactMap { columnNames.firstIndex(of: $0) }
        guard pkIndexes.count == primaryKey.count else {
            error = "Primary key columns are missing from the result"
            return
        }

        typealias Assignment = (column: String, value: String?)
        var updates: [(key: [String?], values: [Assignment])] = []
        var deletes: [[String?]] = []
        var inserts: [[Assignment]] = []

        let editedRows = Dictionary(grouping: edits.keys, by: \.row)
        for row in 0..<rows.count {
            if deletedRows.contains(row) {
                deletes.append(pkIndexes.map { rows[row][$0] })
                continue
            }
            let keys = (editedRows[row] ?? []).sorted { $0.column < $1.column }
            let values = keys.map { (column: columnNames[$0.column], value: edits[$0]!) }
            if isInserted(row: row) {
                inserts.append(values)
            } else if !values.isEmpty {
                updates.append((key: pkIndexes.map { rows[row][$0] }, values: values))
            }
        }

        isLoading = true
        do {
            try await session.run { driver in
                func literal(_ value: String?) -> String { value.map(driver.quoteLiteral) ?? "NULL" }
                func condition(_ key: [String?]) -> String {
                    zip(primaryKey, key).map { column, value in
                        let name = kind.quoteIdentifier(column)
                        return value == nil ? "\(name) IS NULL" : "\(name) = \(literal(value))"
                    }.joined(separator: " AND ")
                }

                var statements: [String] = []
                for key in deletes {
                    statements.append("DELETE FROM \(name) WHERE \(condition(key))")
                }
                for update in updates {
                    let set = update.values
                        .map { "\(kind.quoteIdentifier($0.column)) = \(literal($0.value))" }
                        .joined(separator: ", ")
                    statements.append("UPDATE \(name) SET \(set) WHERE \(condition(update.key))")
                }
                for values in inserts {
                    if values.isEmpty {
                        statements.append(kind == .postgres
                            ? "INSERT INTO \(name) DEFAULT VALUES"
                            : "INSERT INTO \(name) () VALUES ()")
                    } else {
                        let columns = values.map { kind.quoteIdentifier($0.column) }.joined(separator: ", ")
                        let literals = values.map { literal($0.value) }.joined(separator: ", ")
                        statements.append("INSERT INTO \(name) (\(columns)) VALUES (\(literals))")
                    }
                }

                _ = try driver.execute(kind == .mysql ? "START TRANSACTION" : "BEGIN")
                do {
                    for statement in statements { _ = try driver.execute(statement) }
                    _ = try driver.execute("COMMIT")
                } catch {
                    _ = try? driver.execute("ROLLBACK")
                    throw error
                }
            }
            await reload()
        } catch {
            isLoading = false
            self.error = error.localizedDescription
        }
    }
}
