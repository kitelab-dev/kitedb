import SwiftUI

struct QueryTabView: View {
    @Bindable var model: QueryTabModel
    let completionWords: [String]

    var body: some View {
        VSplitView {
            VStack(spacing: 0) {
                SQLEditor(text: $model.sql, selectedText: $model.selectedSQL, completionWords: completionWords)
                Divider()
                runBar
            }
            .frame(minHeight: 140, idealHeight: 280)

            VStack(spacing: 0) {
                results
                Divider()
                statusBar
            }
            .frame(minHeight: 140)
        }
    }

    private var runBar: some View {
        HStack(spacing: 10) {
            if model.isRunning {
                Button {
                    model.cancel()
                } label: {
                    Label("Cancel", systemImage: "stop.fill")
                }
                ProgressView().controlSize(.small)
            } else {
                Button {
                    Task { await model.run() }
                } label: {
                    Label(model.selectedSQL.isEmpty ? "Run" : "Run Selection", systemImage: "play.fill")
                }
                .buttonStyle(.borderedProminent)
                .help("Run (⌘↩)")
                .disabled(model.sql.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            Spacer()
            Text("⌘↩ run · Esc complete")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
        .controlSize(.small)
        .padding(.horizontal, 10)
        .frame(height: 34)
        .background(.bar)
    }

    @ViewBuilder
    private var results: some View {
        if let error = model.error {
            ScrollView {
                Label {
                    Text(error)
                        .font(.system(size: 12, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                } icon: {
                    Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.red)
                }
                .padding(12)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let result = model.result {
            DataGrid(
                columns: result.columns,
                rowCount: result.rows.count,
                version: model.resultVersion,
                cell: { row, column in GridCell(value: result.rows[row][column]) },
                selection: $model.selectedRows
            )
        } else {
            VStack(spacing: 6) {
                Image(systemName: model.message == nil ? "text.cursor" : "checkmark.circle")
                    .font(.system(size: 28))
                    .foregroundStyle(.tertiary)
                Text(model.message == nil ? "Write a query and press ⌘↩" : "Statement executed")
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private var statusBar: some View {
        HStack {
            if let message = model.message {
                Text(message)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
            Spacer()
            if let result = model.result {
                Button {
                    CSVExport.save(columns: result.columns, rows: result.rows, name: "query")
                } label: {
                    Image(systemName: "square.and.arrow.up")
                }
                .buttonStyle(.borderless)
                .help("Export results as CSV")
            }
        }
        .controlSize(.small)
        .padding(.horizontal, 10)
        .frame(height: 28)
        .background(.bar)
    }
}

enum CSVExport {
    @MainActor
    static func save(columns: [String], rows: [[String?]], name: String) {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "\(name).csv"
        panel.allowedContentTypes = [.commaSeparatedText]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        var lines = [columns.map(escape).joined(separator: ",")]
        for row in rows {
            lines.append(row.map { $0.map(escape) ?? "" }.joined(separator: ","))
        }
        try? (lines.joined(separator: "\n") + "\n").write(to: url, atomically: true, encoding: .utf8)
    }

    private static func escape(_ value: String) -> String {
        guard value.contains(where: { $0 == "," || $0 == "\"" || $0 == "\n" || $0 == "\r" }) else { return value }
        return "\"" + value.replacingOccurrences(of: "\"", with: "\"\"") + "\""
    }
}
