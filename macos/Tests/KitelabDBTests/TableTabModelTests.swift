import XCTest
@testable import KitelabDB

@MainActor
final class TableTabModelTests: XCTestCase {
    func testStagedChangesAreSavedInOneTransaction() async throws {
        guard let spec = ProcessInfo.processInfo.environment["KITELAB_MYSQL"] else {
            throw XCTSkip("KITELAB_MYSQL not set")
        }
        let parts = spec.split(separator: ":", omittingEmptySubsequences: false).map(String.init)
        var config = ConnectionConfig()
        config.host = parts[0]
        config.port = Int(parts[1]) ?? 3306
        config.user = parts[2]
        let session = DBSession(driver: DriverFactory.make(config: config, password: parts.count > 3 ? parts[3] : ""))
        defer { session.close() }
        try await session.run { driver in
            try driver.connect()
            _ = try driver.execute("""
                DROP DATABASE IF EXISTS kitelab_model_test; CREATE DATABASE kitelab_model_test;
                USE kitelab_model_test;
                CREATE TABLE items (id INT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(20) NOT NULL, note TEXT NULL);
                INSERT INTO items (name, note) VALUES ('a', 'x'), ('b', NULL), ('c', 'z');
                """)
            try driver.useDatabase("kitelab_model_test")
        }

        let model = TableTabModel(table: TableRef(name: "items"), session: session, kind: .mysql)
        await model.load()
        XCTAssertNil(model.error)
        XCTAssertTrue(model.canEdit)
        XCTAssertEqual(model.rows.count, 3)

        model.setValue("a2", row: 0, column: 1)
        model.setValue(nil, row: 0, column: 2)
        model.setValue("b", row: 1, column: 1) // unchanged value: no edit
        model.deleteRows(IndexSet(integer: 2))
        model.addRow()
        model.setValue("d", row: 3, column: 1)
        XCTAssertEqual(model.changeCount, 3)
        await model.saveChanges()
        XCTAssertNil(model.error)
        XCTAssertFalse(model.hasChanges)
        XCTAssertEqual(model.rows.map { $0[1] }, ["a2", "b", "d"])
        XCTAssertEqual(model.rows[0][2], nil)

        // A failing statement rolls the whole batch back.
        model.setValue("zzz", row: 1, column: 1)
        model.addRow() // name is NOT NULL without default -> fails
        model.setValue(nil, row: 3, column: 1)
        await model.saveChanges()
        XCTAssertNotNil(model.error)
        await model.reload()
        XCTAssertEqual(model.rows.map { $0[1] }, ["a2", "b", "d"])

        model.filter = TableFilter(column: "name", op: .contains, value: "2")
        await model.applyFilter()
        XCTAssertEqual(model.rows.map { $0[1] }, ["a2"])
        model.filter = TableFilter(column: "", value: "id >= 2")
        await model.applyFilter()
        await model.sort(by: 0)
        await model.sort(by: 0)
        XCTAssertEqual(model.rows.map { $0[1] }, ["d", "b"])

        _ = try await session.run { try $0.execute("DROP DATABASE kitelab_model_test") }
    }
}
