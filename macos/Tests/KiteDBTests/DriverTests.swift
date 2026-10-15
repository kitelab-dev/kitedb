import XCTest
@testable import KiteDB

/// Integration tests against live servers. Configure with environment
/// variables; a test is skipped when its server variables are missing:
///   KITELAB_MYSQL=host:port:user:password
///   KITELAB_PG=host:port:user:password
final class DriverTests: XCTestCase {
    private func makeDriver(_ kind: DatabaseKind, env name: String) throws -> DatabaseDriver {
        guard let spec = ProcessInfo.processInfo.environment[name] else {
            throw XCTSkip("\(name) not set")
        }
        let parts = spec.split(separator: ":", omittingEmptySubsequences: false).map(String.init)
        var config = ConnectionConfig()
        config.kind = kind
        config.host = parts[0]
        config.port = Int(parts[1]) ?? kind.defaultPort
        config.user = parts[2]
        config.sslMode = .prefer
        let driver = DriverFactory.make(config: config, password: parts.count > 3 ? parts[3] : "")
        try driver.connect()
        return driver
    }

    func testMySQL() throws {
        let driver = try makeDriver(.mysql, env: "KITELAB_MYSQL")
        defer { driver.close() }
        XCTAssertFalse(driver.serverVersion.isEmpty)
        _ = try driver.execute("DROP DATABASE IF EXISTS kitelab_test; CREATE DATABASE kitelab_test")
        try driver.useDatabase("kitelab_test")
        XCTAssertTrue(try driver.databases().contains("kitelab_test"))
        _ = try driver.execute("""
            CREATE TABLE users (id INT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(50) NOT NULL,
              bio TEXT NULL, avatar VARBINARY(4) NULL, score DECIMAL(6,2) DEFAULT 1.5, KEY idx_name (name));
            CREATE VIEW user_names AS SELECT name FROM users;
            """)
        let results = try driver.execute("""
            INSERT INTO users (name, bio, avatar) VALUES ('Бат', NULL, 0x0102), ('O''Neil', 'a\\\\b', NULL);
            SELECT * FROM users ORDER BY id
            """)
        XCTAssertEqual(results.count, 2)
        XCTAssertEqual(results[0].affectedRows, 2)
        let rows = results[1]
        XCTAssertEqual(rows.columns, ["id", "name", "bio", "avatar", "score"])
        XCTAssertEqual(rows.rows[0], ["1", "Бат", nil, "0x0102", "1.50"])
        XCTAssertEqual(rows.rows[1][1], "O'Neil")
        XCTAssertEqual(rows.rows[1][2], "a\\b")

        XCTAssertEqual(driver.quoteLiteral("it's \\ ok"), "'it\\'s \\\\ ok'")
        let quoted = try driver.query("SELECT \(driver.quoteLiteral("it's \\ ok"))")
        XCTAssertEqual(quoted.rows[0][0], "it's \\ ok")

        let tables = try driver.tables(schema: nil)
        XCTAssertEqual(tables.map(\.name), ["user_names", "users"])
        XCTAssertEqual(tables.map(\.kind), [.view, .table])
        let columns = try driver.columns(of: TableRef(name: "users"))
        XCTAssertEqual(columns.filter(\.isPrimaryKey).map(\.name), ["id"])
        XCTAssertEqual(columns[4].defaultValue, "1.50")
        let indexes = try driver.indexes(of: TableRef(name: "users"))
        XCTAssertEqual(indexes.map(\.name), ["PRIMARY", "idx_name"])

        XCTAssertThrowsError(try driver.execute("SELEC 1"))
        // The connection is still usable after an error.
        XCTAssertEqual(try driver.query("SELECT 1 + 1").rows[0][0], "2")
        _ = try driver.execute("DROP DATABASE kitelab_test")
    }

    func testPostgres() throws {
        let driver = try makeDriver(.postgres, env: "KITELAB_PG")
        defer { driver.close() }
        _ = try driver.execute("DROP SCHEMA IF EXISTS kitelab_test CASCADE; CREATE SCHEMA kitelab_test")
        _ = try driver.execute("""
            CREATE TABLE kitelab_test.users (id serial PRIMARY KEY, name text NOT NULL, tags text[],
              meta jsonb, created timestamptz DEFAULT now(), raw bytea);
            CREATE INDEX idx_name ON kitelab_test.users (name);
            CREATE VIEW kitelab_test.user_names AS SELECT name FROM kitelab_test.users;
            """)
        let results = try driver.execute("""
            INSERT INTO kitelab_test.users (name, tags, meta, raw) VALUES ('Бат', '{a,b}', '{"k": 1}', '\\x0102'), ('O''Neil', NULL, NULL, NULL);
            SELECT id, name, tags, meta, raw FROM kitelab_test.users ORDER BY id
            """)
        XCTAssertEqual(results[0].affectedRows, 2)
        XCTAssertEqual(results[1].rows[0], ["1", "Бат", "{a,b}", "{\"k\": 1}", "\\x0102"])
        XCTAssertEqual(results[1].rows[1][2], nil)

        XCTAssertTrue(try driver.schemas().contains("kitelab_test"))
        let tables = try driver.tables(schema: "kitelab_test")
        XCTAssertEqual(tables.map(\.name), ["user_names", "users"])
        let users = TableRef(schema: "kitelab_test", name: "users")
        let columns = try driver.columns(of: users)
        XCTAssertEqual(columns.filter(\.isPrimaryKey).map(\.name), ["id"])
        XCTAssertEqual(columns[1].type, "text")
        let indexes = try driver.indexes(of: users)
        XCTAssertEqual(indexes.map(\.name), ["users_pkey", "idx_name"])

        XCTAssertEqual(try driver.query("SELECT \(driver.quoteLiteral("it's"))").rows[0][0], "it's")
        XCTAssertThrowsError(try driver.execute("SELEC 1"))
        XCTAssertEqual(try driver.query("SELECT 1 + 1").rows[0][0], "2")
        _ = try driver.execute("DROP SCHEMA kitelab_test CASCADE")
    }
}
