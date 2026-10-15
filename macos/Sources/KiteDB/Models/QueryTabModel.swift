import Foundation
import Observation

@MainActor
@Observable
final class QueryTabModel {
    let title: String
    let session: DBSession

    var sql = ""
    /// Text currently selected in the editor; when non-empty only it runs.
    var selectedSQL = ""

    private(set) var result: QueryResult?
    private(set) var resultVersion = 0
    private(set) var message: String?
    private(set) var error: String?
    private(set) var isRunning = false
    var selectedRows = IndexSet()

    init(title: String, session: DBSession) {
        self.title = title
        self.session = session
    }

    func run() async {
        let selection = selectedSQL.trimmingCharacters(in: .whitespacesAndNewlines)
        let text = selection.isEmpty ? sql.trimmingCharacters(in: .whitespacesAndNewlines) : selection
        guard !text.isEmpty, !isRunning else { return }

        isRunning = true
        error = nil
        message = nil
        defer { isRunning = false }
        let started = Date()
        do {
            let results = try await session.run { try $0.execute(text) }
            let elapsed = Date().timeIntervalSince(started)
            let shown = results.last(where: \.hasResultSet) ?? results.last
            result = shown?.hasResultSet == true ? shown : nil
            resultVersion += 1
            message = Self.summary(results: results, shown: shown, elapsed: elapsed)
        } catch {
            result = nil
            resultVersion += 1
            self.error = error.localizedDescription
        }
    }

    func cancel() {
        guard isRunning else { return }
        session.cancel()
    }

    private static func summary(results: [QueryResult], shown: QueryResult?, elapsed: TimeInterval) -> String {
        var parts: [String] = []
        if let shown, shown.hasResultSet {
            let count = shown.rows.count
            parts.append("\(count) row\(count == 1 ? "" : "s")")
            if shown.truncated { parts.append("showing first \(count)") }
        } else {
            let affected = results.compactMap(\.affectedRows).reduce(0, +)
            parts.append("Query OK, \(affected) row\(affected == 1 ? "" : "s") affected")
        }
        if results.count > 1 { parts.append("\(results.count) statements") }
        parts.append(formatDuration(elapsed))
        return parts.joined(separator: " · ")
    }
}

func formatDuration(_ interval: TimeInterval) -> String {
    interval < 1 ? "\(Int((interval * 1000).rounded())) ms" : String(format: "%.2f s", interval)
}
