import Foundation

struct DBError: LocalizedError {
    let message: String
    /// Native error code (MySQL errno, unused for Postgres).
    var code: Int = 0

    init(_ message: String, code: Int = 0) {
        self.message = message.trimmingCharacters(in: .whitespacesAndNewlines)
        self.code = code
    }

    var errorDescription: String? { message }
}

/// One result of a statement. Values are kept in the server's text
/// representation, which is exactly what a client UI needs to display.
struct QueryResult {
    var columns: [String] = []
    var rows: [[String?]] = []
    /// Rows affected by INSERT/UPDATE/DELETE, when the server reports it.
    var affectedRows: Int?
    /// True when more rows were returned than `DatabaseDriver.maxRows`.
    var truncated = false

    var hasResultSet: Bool { !columns.isEmpty }
}

struct TableRef: Hashable, Identifiable {
    enum Kind: Hashable { case table, view }

    var schema: String?
    var name: String
    var kind: Kind = .table

    var id: String { "\(schema ?? "").\(name)" }
}

struct ColumnInfo: Identifiable, Hashable {
    var name: String
    var type: String
    var nullable: Bool
    var defaultValue: String?
    var isPrimaryKey: Bool
    var extra: String
    var comment: String

    var id: String { name }
}

struct IndexInfo: Identifiable, Hashable {
    var name: String
    var columns: String
    var unique: Bool
    var primary: Bool
    var definition: String

    var id: String { name }
}

/// A blocking, single-connection database driver. Every call except
/// `cancel()` must happen on the owning `DBSession` queue.
protocol DatabaseDriver: AnyObject {
    var kind: DatabaseKind { get }
    var serverVersion: String { get }
    /// The database the connection currently points to.
    var currentDatabase: String? { get }

    func connect() throws
    func close()
    /// Runs one or more statements and returns every result produced.
    func execute(_ sql: String) throws -> [QueryResult]
    func quoteLiteral(_ value: String) -> String
    /// Thread-safe: may be called while a query runs on the session queue.
    func cancel()

    func databases() throws -> [String]
    func useDatabase(_ name: String) throws
    func schemas() throws -> [String]
    func tables(schema: String?) throws -> [TableRef]
    func columns(of table: TableRef) throws -> [ColumnInfo]
    func indexes(of table: TableRef) throws -> [IndexInfo]
}

extension DatabaseDriver {
    static var maxRows: Int { 20_000 }

    func quoteIdentifier(_ name: String) -> String { kind.quoteIdentifier(name) }

    func qualifiedName(_ table: TableRef) -> String { kind.qualifiedName(table) }

    /// Runs a query and returns the last result that carried rows (or the
    /// last result overall).
    func query(_ sql: String) throws -> QueryResult {
        let results = try execute(sql)
        return results.last(where: \.hasResultSet) ?? results.last ?? QueryResult()
    }
}

extension DatabaseKind {
    func quoteIdentifier(_ name: String) -> String {
        switch self {
        case .mysql: "`" + name.replacingOccurrences(of: "`", with: "``") + "`"
        case .postgres: "\"" + name.replacingOccurrences(of: "\"", with: "\"\"") + "\""
        }
    }

    func qualifiedName(_ table: TableRef) -> String {
        if let schema = table.schema, self == .postgres {
            return quoteIdentifier(schema) + "." + quoteIdentifier(table.name)
        }
        return quoteIdentifier(table.name)
    }
}

/// Owns a driver and serializes all access to it on a private queue, so the
/// blocking C client libraries never run on the main thread.
final class DBSession: @unchecked Sendable {
    let driver: DatabaseDriver
    private let queue: DispatchQueue

    init(driver: DatabaseDriver) {
        self.driver = driver
        self.queue = DispatchQueue(label: "kitelab.db.session.\(UUID().uuidString)", qos: .userInitiated)
    }

    func run<T>(_ work: @escaping (DatabaseDriver) throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                continuation.resume(with: Result { try work(self.driver) })
            }
        }
    }

    func cancel() {
        DispatchQueue.global().async { self.driver.cancel() }
    }

    func close() {
        queue.async { self.driver.close() }
    }
}

enum DriverFactory {
    static func make(config: ConnectionConfig, password: String) -> DatabaseDriver {
        switch config.kind {
        case .mysql: MySQLDriver(config: config, password: password)
        case .postgres: PostgresDriver(config: config, password: password)
        }
    }
}
