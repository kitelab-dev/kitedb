import CLibPQ
import Foundation

final class PostgresDriver: DatabaseDriver {
    let kind = DatabaseKind.postgres

    private let config: ConnectionConfig
    private let password: String
    private var database: String
    private var conn: OpaquePointer?
    private var cancelHandle: OpaquePointer?
    private let cancelLock = NSLock()

    init(config: ConnectionConfig, password: String) {
        self.config = config
        self.password = password
        self.database = config.database.isEmpty ? "postgres" : config.database
    }

    deinit { close() }

    var serverVersion: String {
        guard let conn, let value = PQparameterStatus(conn, "server_version") else { return "" }
        return String(cString: value)
    }

    var currentDatabase: String? { database }

    func connect() throws {
        close()
        let params: [(String, String)] = [
            ("host", config.host),
            ("port", String(config.port)),
            ("user", config.user),
            ("password", password),
            ("dbname", database),
            ("sslmode", config.sslMode.rawValue),
            ("connect_timeout", "10"),
            ("application_name", "Kitelab DB"),
        ]
        let keys = params.map { strdup($0.0) }
        let values = params.map { strdup($0.1) }
        defer {
            keys.forEach { free($0) }
            values.forEach { free($0) }
        }
        var keyPointers = keys.map { UnsafePointer<CChar>($0) } + [nil]
        var valuePointers = values.map { UnsafePointer<CChar>($0) } + [nil]
        let connection = keyPointers.withUnsafeMutableBufferPointer { k in
            valuePointers.withUnsafeMutableBufferPointer { v in
                PQconnectdbParams(k.baseAddress, v.baseAddress, 0)
            }
        }
        guard let connection, PQstatus(connection) == CONNECTION_OK else {
            let message = connection.map { String(cString: PQerrorMessage($0)) } ?? "Could not allocate connection"
            if let connection { PQfinish(connection) }
            throw DBError(message)
        }
        PQsetNoticeProcessor(connection, { _, _ in }, nil)
        conn = connection
        cancelLock.withLock { cancelHandle = PQgetCancel(connection) }
    }

    func close() {
        cancelLock.withLock {
            if let cancelHandle { PQfreeCancel(cancelHandle) }
            cancelHandle = nil
        }
        if let conn { PQfinish(conn) }
        conn = nil
    }

    func cancel() {
        cancelLock.withLock {
            guard let cancelHandle else { return }
            var buffer = [CChar](repeating: 0, count: 256)
            _ = PQcancel(cancelHandle, &buffer, Int32(buffer.count))
        }
    }

    private func ensureConnected() throws -> OpaquePointer {
        if let conn, PQstatus(conn) == CONNECTION_OK { return conn }
        try connect()
        return conn!
    }

    func execute(_ sql: String) throws -> [QueryResult] {
        var conn = try ensureConnected()
        if PQsendQuery(conn, sql) == 0 {
            // The server may have dropped an idle connection; retry once.
            try connect()
            conn = self.conn!
            guard PQsendQuery(conn, sql) == 1 else {
                throw DBError(String(cString: PQerrorMessage(conn)))
            }
        }

        var results: [QueryResult] = []
        var error: DBError?
        while let res = PQgetResult(conn) {
            defer { PQclear(res) }
            let status = PQresultStatus(res)
            if status == PGRES_TUPLES_OK || status == PGRES_SINGLE_TUPLE {
                results.append(readResult(res))
            } else if status == PGRES_COMMAND_OK {
                var result = QueryResult()
                result.affectedRows = Int(String(cString: PQcmdTuples(res)))
                results.append(result)
            } else if status == PGRES_EMPTY_QUERY {
                continue
            } else if error == nil {
                let message = String(cString: PQresultErrorMessage(res))
                if !message.isEmpty { error = DBError(message) }
            }
        }
        if let error { throw error }
        return results
    }

    private func readResult(_ res: OpaquePointer) -> QueryResult {
        var result = QueryResult()
        let fieldCount = PQnfields(res)
        let rowCount = Int(PQntuples(res))
        result.columns = (0..<fieldCount).map { String(cString: PQfname(res, $0)) }
        let limit = min(rowCount, Self.maxRows)
        result.truncated = rowCount > limit
        result.rows.reserveCapacity(limit)
        for row in 0..<Int32(limit) {
            var values: [String?] = []
            values.reserveCapacity(Int(fieldCount))
            for column in 0..<fieldCount {
                if PQgetisnull(res, row, column) == 1 {
                    values.append(nil)
                } else {
                    values.append(String(cString: PQgetvalue(res, row, column)))
                }
            }
            result.rows.append(values)
        }
        return result
    }

    func quoteLiteral(_ value: String) -> String {
        if let conn, let escaped = PQescapeLiteral(conn, value, value.utf8.count) {
            defer { PQfreemem(escaped) }
            return String(cString: escaped)
        }
        return "'" + value.replacingOccurrences(of: "'", with: "''") + "'"
    }

    // MARK: - Introspection

    func databases() throws -> [String] {
        try query("SELECT datname FROM pg_database WHERE datallowconn AND NOT datistemplate ORDER BY datname")
            .rows.compactMap { $0[0] }
    }

    func useDatabase(_ name: String) throws {
        guard name != database else { return }
        let previous = database
        database = name
        do {
            try connect()
        } catch {
            database = previous
            try? connect()
            throw error
        }
    }

    func schemas() throws -> [String] {
        try query("""
            SELECT nspname FROM pg_namespace
            WHERE nspname NOT LIKE 'pg\\_%' AND nspname <> 'information_schema'
            ORDER BY nspname = 'public' DESC, nspname
            """).rows.compactMap { $0[0] }
    }

    func tables(schema: String?) throws -> [TableRef] {
        let schema = schema ?? "public"
        return try query("""
            SELECT c.relname, c.relkind FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname = \(quoteLiteral(schema)) AND c.relkind IN ('r', 'p', 'f', 'v', 'm')
            ORDER BY c.relname
            """).rows.map { row in
            let kind: TableRef.Kind = (row[1] == "v" || row[1] == "m") ? .view : .table
            return TableRef(schema: schema, name: row[0] ?? "", kind: kind)
        }
    }

    private func relationOID(_ table: TableRef) -> String {
        "(SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace "
            + "WHERE n.nspname = \(quoteLiteral(table.schema ?? "public")) AND c.relname = \(quoteLiteral(table.name)))"
    }

    func columns(of table: TableRef) throws -> [ColumnInfo] {
        try query("""
            SELECT a.attname,
                   format_type(a.atttypid, a.atttypmod),
                   NOT a.attnotnull,
                   pg_get_expr(d.adbin, d.adrelid),
                   EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = a.attrelid AND i.indisprimary AND a.attnum = ANY(i.indkey)),
                   CASE WHEN a.attidentity <> '' THEN 'identity' WHEN a.attgenerated <> '' THEN 'generated' ELSE '' END,
                   col_description(a.attrelid, a.attnum)
            FROM pg_attribute a
            LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
            WHERE a.attrelid = \(relationOID(table)) AND a.attnum > 0 AND NOT a.attisdropped
            ORDER BY a.attnum
            """).rows.map { row in
            ColumnInfo(
                name: row[0] ?? "",
                type: row[1] ?? "",
                nullable: row[2] == "t",
                defaultValue: row[3],
                isPrimaryKey: row[4] == "t",
                extra: row[5] ?? "",
                comment: row[6] ?? ""
            )
        }
    }

    func indexes(of table: TableRef) throws -> [IndexInfo] {
        try query("""
            SELECT i.relname,
                   (SELECT string_agg(pg_get_indexdef(ix.indexrelid, k, true), ', ' ORDER BY k)
                    FROM generate_series(1, ix.indnkeyatts) AS k),
                   ix.indisunique, ix.indisprimary, pg_get_indexdef(ix.indexrelid)
            FROM pg_index ix
            JOIN pg_class i ON i.oid = ix.indexrelid
            WHERE ix.indrelid = \(relationOID(table))
            ORDER BY ix.indisprimary DESC, i.relname
            """).rows.map { row in
            IndexInfo(
                name: row[0] ?? "",
                columns: row[1] ?? "",
                unique: row[2] == "t",
                primary: row[3] == "t",
                definition: row[4] ?? ""
            )
        }
    }
}
