import CMySQL
import Foundation

final class MySQLDriver: DatabaseDriver {
    let kind = DatabaseKind.mysql

    private let config: ConnectionConfig
    private let password: String
    private var database: String?
    private var mysql: UnsafeMutablePointer<MYSQL>?
    private var threadID: UInt = 0
    private let stateLock = NSLock()

    /// Client errors meaning the connection is gone and worth re-opening.
    private static let connectionLostErrors: Set<UInt32> = [2006, 2013, 4031]

    private static let binaryTypes: Set<UInt32> = [
        MYSQL_TYPE_TINY_BLOB.rawValue, MYSQL_TYPE_MEDIUM_BLOB.rawValue, MYSQL_TYPE_LONG_BLOB.rawValue,
        MYSQL_TYPE_BLOB.rawValue, MYSQL_TYPE_STRING.rawValue, MYSQL_TYPE_VAR_STRING.rawValue,
        MYSQL_TYPE_VARCHAR.rawValue, MYSQL_TYPE_BIT.rawValue, MYSQL_TYPE_GEOMETRY.rawValue,
    ]

    init(config: ConnectionConfig, password: String) {
        self.config = config
        self.password = password
        self.database = config.database.isEmpty ? nil : config.database
    }

    deinit { close() }

    var serverVersion: String {
        guard let mysql, let info = mysql_get_server_info(mysql) else { return "" }
        return String(cString: info)
    }

    var currentDatabase: String? { database }

    func connect() throws {
        close()
        let handle = try Self.open(config: config, password: password, database: database)
        mysql = handle
        stateLock.withLock { threadID = mysql_thread_id(handle) }
    }

    private static func open(config: ConnectionConfig, password: String, database: String?) throws -> UnsafeMutablePointer<MYSQL> {
        guard let handle = mysql_init(nil) else { throw DBError("Could not allocate MySQL client") }
        var timeout: UInt32 = 10
        mysql_options(handle, MYSQL_OPT_CONNECT_TIMEOUT, &timeout)
        var sslMode: UInt32 = switch config.sslMode {
        case .disable: SSL_MODE_DISABLED.rawValue
        case .prefer: SSL_MODE_PREFERRED.rawValue
        case .require: SSL_MODE_REQUIRED.rawValue
        }
        mysql_options(handle, MYSQL_OPT_SSL_MODE, &sslMode)
        mysql_options(handle, MYSQL_SET_CHARSET_NAME, "utf8mb4")

        let flags = UInt(CLIENT_MULTI_STATEMENTS) | UInt(CLIENT_MULTI_RESULTS)
        let connected = mysql_real_connect(
            handle, config.host, config.user, password, database,
            UInt32(config.port), nil, flags
        )
        guard connected != nil else {
            let error = DBError(String(cString: mysql_error(handle)), code: Int(mysql_errno(handle)))
            mysql_close(handle)
            throw error
        }
        return handle
    }

    func close() {
        if let mysql { mysql_close(mysql) }
        mysql = nil
    }

    func cancel() {
        let id = stateLock.withLock { threadID }
        guard id != 0, let killer = try? Self.open(config: config, password: password, database: nil) else { return }
        _ = mysql_query(killer, "KILL QUERY \(id)")
        mysql_close(killer)
    }

    private func ensureConnected() throws -> UnsafeMutablePointer<MYSQL> {
        if let mysql { return mysql }
        try connect()
        return mysql!
    }

    func execute(_ sql: String) throws -> [QueryResult] {
        do {
            return try run(sql)
        } catch let error as DBError where Self.connectionLostErrors.contains(UInt32(error.code)) {
            try connect()
            return try run(sql)
        }
    }

    private func run(_ sql: String) throws -> [QueryResult] {
        let mysql = try ensureConnected()
        let bytes = Array(sql.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            buffer.withMemoryRebound(to: CChar.self) { mysql_real_query(mysql, $0.baseAddress, UInt(bytes.count)) }
        }
        guard status == 0 else { throw lastError(mysql) }

        var results: [QueryResult] = []
        while true {
            if let res = mysql_store_result(mysql) {
                results.append(readResult(res))
                mysql_free_result(res)
            } else if mysql_field_count(mysql) == 0 {
                var result = QueryResult()
                result.affectedRows = Int(mysql_affected_rows(mysql))
                results.append(result)
            } else {
                throw lastError(mysql)
            }
            let next = mysql_next_result(mysql)
            if next == -1 { break }
            if next > 0 { throw lastError(mysql) }
        }
        return results
    }

    private func lastError(_ mysql: UnsafeMutablePointer<MYSQL>) -> DBError {
        DBError(String(cString: mysql_error(mysql)), code: Int(mysql_errno(mysql)))
    }

    private func readResult(_ res: UnsafeMutablePointer<MYSQL_RES>) -> QueryResult {
        var result = QueryResult()
        let fieldCount = Int(mysql_num_fields(res))
        let fields = mysql_fetch_fields(res)!
        var isBinary = [Bool](repeating: false, count: fieldCount)
        for i in 0..<fieldCount {
            let field = fields[i]
            result.columns.append(String(cString: field.name))
            isBinary[i] = field.charsetnr == 63 && Self.binaryTypes.contains(field.type.rawValue)
        }

        let rowCount = Int(mysql_num_rows(res))
        result.truncated = rowCount > Self.maxRows
        result.rows.reserveCapacity(min(rowCount, Self.maxRows))
        while result.rows.count < Self.maxRows, let row = mysql_fetch_row(res) {
            let lengths = mysql_fetch_lengths(res)!
            var values: [String?] = []
            values.reserveCapacity(fieldCount)
            for i in 0..<fieldCount {
                guard let pointer = row[i] else {
                    values.append(nil)
                    continue
                }
                let buffer = UnsafeRawBufferPointer(start: pointer, count: Int(lengths[i]))
                values.append(isBinary[i] ? Self.hex(buffer) : String(decoding: buffer, as: UTF8.self))
            }
            result.rows.append(values)
        }
        return result
    }

    private static func hex(_ bytes: UnsafeRawBufferPointer) -> String {
        let limit = 512
        var text = "0x"
        text.reserveCapacity(2 + min(bytes.count, limit) * 2)
        for byte in bytes.prefix(limit) {
            text += String(format: "%02X", byte)
        }
        if bytes.count > limit { text += "…" }
        return text
    }

    func quoteLiteral(_ value: String) -> String {
        guard let mysql else {
            let escaped = value
                .replacingOccurrences(of: "\\", with: "\\\\")
                .replacingOccurrences(of: "'", with: "\\'")
            return "'\(escaped)'"
        }
        let source = Array(value.utf8CString)
        var target = [CChar](repeating: 0, count: source.count * 2 + 1)
        let length = mysql_real_escape_string_quote(mysql, &target, source, UInt(source.count - 1), CChar(UInt8(ascii: "'")))
        let escaped = target.prefix(Int(length)).map { UInt8(bitPattern: $0) }
        return "'" + String(decoding: escaped, as: UTF8.self) + "'"
    }

    // MARK: - Introspection

    func databases() throws -> [String] {
        try query("SHOW DATABASES").rows.compactMap { $0[0] }
    }

    func useDatabase(_ name: String) throws {
        let mysql = try ensureConnected()
        guard mysql_select_db(mysql, name) == 0 else { throw lastError(mysql) }
        database = name
    }

    func schemas() throws -> [String] { [] }

    func tables(schema: String?) throws -> [TableRef] {
        guard database != nil else { return [] }
        return try query("""
            SELECT TABLE_NAME, TABLE_TYPE FROM information_schema.TABLES
            WHERE TABLE_SCHEMA = DATABASE() ORDER BY TABLE_NAME
            """).rows.map { row in
            TableRef(schema: nil, name: row[0] ?? "", kind: row[1] == "VIEW" ? .view : .table)
        }
    }

    func columns(of table: TableRef) throws -> [ColumnInfo] {
        try query("""
            SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, COLUMN_KEY, EXTRA, COLUMN_COMMENT
            FROM information_schema.COLUMNS
            WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = \(quoteLiteral(table.name))
            ORDER BY ORDINAL_POSITION
            """).rows.map { row in
            ColumnInfo(
                name: row[0] ?? "",
                type: row[1] ?? "",
                nullable: row[2] == "YES",
                defaultValue: row[3],
                isPrimaryKey: row[4] == "PRI",
                extra: row[5] ?? "",
                comment: row[6] ?? ""
            )
        }
    }

    func indexes(of table: TableRef) throws -> [IndexInfo] {
        try query("""
            SELECT INDEX_NAME,
                   GROUP_CONCAT(COLUMN_NAME ORDER BY SEQ_IN_INDEX SEPARATOR ', '),
                   MIN(NON_UNIQUE), INDEX_TYPE
            FROM information_schema.STATISTICS
            WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = \(quoteLiteral(table.name))
            GROUP BY INDEX_NAME, INDEX_TYPE
            ORDER BY INDEX_NAME = 'PRIMARY' DESC, INDEX_NAME
            """).rows.map { row in
            IndexInfo(
                name: row[0] ?? "",
                columns: row[1] ?? "",
                unique: row[2] == "0",
                primary: row[0] == "PRIMARY",
                definition: row[3] ?? ""
            )
        }
    }
}
