import SwiftUI

struct TableTabView: View {
    @Bindable var model: TableTabModel

    var body: some View {
        VStack(spacing: 0) {
            switch model.mode {
            case .data:
                FilterBar(model: model)
                Divider()
                grid
            case .structure:
                StructureView(model: model)
            }
            if let error = model.error {
                ErrorBanner(message: error) { model.error = nil }
            }
            Divider()
            bottomBar
        }
    }

    private var grid: some View {
        DataGrid(
            columns: model.resultColumns,
            rowCount: model.rows.count,
            version: model.dataVersion,
            cell: { row, column in
                GridCell(
                    value: model.value(row: row, column: column),
                    placeholder: model.isInserted(row: row) && !model.isEdited(row: row, column: column) ? "DEFAULT" : nil,
                    edited: model.isEdited(row: row, column: column) && !model.isInserted(row: row)
                )
            },
            rowState: { row in
                if model.deletedRows.contains(row) { return .deleted }
                return model.isInserted(row: row) ? .inserted : .normal
            },
            editable: model.canEdit,
            sortColumn: model.sortColumn,
            sortAscending: model.sortAscending,
            selection: $model.selectedRows,
            onEdit: { row, column, value in model.setValue(value, row: row, column: column) },
            onSort: { column in
                guard !model.hasChanges else { return }
                Task { await model.sort(by: column) }
            },
            onDelete: { rows in model.deleteRows(rows) }
        )
        .overlay(alignment: .top) {
            if model.isLoading {
                ProgressView()
                    .controlSize(.small)
                    .padding(8)
                    .background(.regularMaterial, in: Capsule())
                    .padding(.top, 30)
            }
        }
    }

    private var bottomBar: some View {
        HStack(spacing: 10) {
            Picker("", selection: $model.mode) {
                ForEach(TableTabModel.Mode.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .fixedSize()

            if model.mode == .data {
                Button {
                    model.addRow()
                } label: {
                    Image(systemName: "plus")
                }
                .help("Add row (⌘I)")
                .disabled(!model.canEdit)
                Button {
                    model.deleteRows(model.selectedRows)
                } label: {
                    Image(systemName: "minus")
                }
                .help("Delete selected rows (⌫)")
                .disabled(!model.canEdit || model.selectedRows.isEmpty)

                if let reason = model.editDisabledReason {
                    Label(reason, systemImage: "lock")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            Spacer()

            if model.hasChanges {
                Text("\(model.changeCount) pending change\(model.changeCount == 1 ? "" : "s")")
                    .font(.caption)
                    .foregroundStyle(.orange)
                Button("Discard") { model.discardChanges() }
                Button("Save") { Task { await model.saveChanges() } }
                    .buttonStyle(.borderedProminent)
                    .help("Commit changes (⌘S)")
            } else if model.mode == .data {
                Text(statusText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }

            if model.mode == .data {
                HStack(spacing: 2) {
                    Button {
                        Task { await model.goToPage(model.page - 1) }
                    } label: {
                        Image(systemName: "chevron.left")
                    }
                    .disabled(model.page == 0 || model.hasChanges)
                    Text("\(model.page + 1)")
                        .font(.caption.monospacedDigit())
                        .frame(minWidth: 22)
                    Button {
                        Task { await model.goToPage(model.page + 1) }
                    } label: {
                        Image(systemName: "chevron.right")
                    }
                    .disabled(!model.hasNextPage || model.hasChanges)
                }
                Button {
                    CSVExport.save(columns: model.resultColumns, rows: model.rows, name: model.table.name)
                } label: {
                    Image(systemName: "square.and.arrow.up")
                }
                .help("Export this page as CSV")
                .disabled(model.rows.isEmpty)
            }

            Button {
                Task {
                    if model.mode == .structure { await model.loadStructure() } else { await model.reload() }
                }
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .help("Reload")
            .disabled(model.hasChanges)
        }
        .buttonStyle(.borderless)
        .controlSize(.small)
        .padding(.horizontal, 10)
        .frame(height: 32)
        .background(.bar)
    }

    private var statusText: String {
        guard model.loadedRowCount > 0 else { return "No rows · \(formatDuration(model.duration))" }
        let first = model.firstRowNumber
        let last = first + model.loadedRowCount - 1
        return "Rows \(first)–\(last) · \(formatDuration(model.duration))"
    }
}

struct FilterBar: View {
    @Bindable var model: TableTabModel

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "line.3.horizontal.decrease")
                .foregroundStyle(.secondary)
            Picker("", selection: $model.filter.column) {
                Text("Raw SQL").tag("")
                Divider()
                ForEach(model.resultColumns.isEmpty ? model.columns.map(\.name) : model.resultColumns, id: \.self) {
                    Text($0).tag($0)
                }
            }
            .labelsHidden()
            .frame(width: 150)

            if !model.filter.isRaw {
                Picker("", selection: $model.filter.op) {
                    ForEach(FilterOperator.allCases) { Text($0.rawValue).tag($0) }
                }
                .labelsHidden()
                .frame(width: 110)
            }

            if model.filter.isRaw || model.filter.op.needsValue {
                TextField(model.filter.isRaw ? "WHERE clause, e.g. id > 10 AND status = 'active'" : "Value",
                          text: $model.filter.value)
                    .textFieldStyle(.roundedBorder)
                    .font(.system(size: 12, design: model.filter.isRaw ? .monospaced : .default))
                    .onSubmit { apply() }
            } else {
                Spacer()
            }

            Button("Apply") { apply() }
                .disabled(model.hasChanges)
            if model.appliedFilter != TableFilter() && !model.appliedFilter.isEmpty {
                Button("Clear") { Task { await model.clearFilter() } }
                    .disabled(model.hasChanges)
            }
        }
        .controlSize(.small)
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
    }

    private func apply() {
        guard !model.hasChanges else { return }
        Task { await model.applyFilter() }
    }
}

struct StructureView: View {
    let model: TableTabModel

    var body: some View {
        VSplitView {
            Table(model.columns) {
                TableColumn("#") { column in
                    Text("\((model.columns.firstIndex(of: column) ?? 0) + 1)")
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                }
                .width(28)
                TableColumn("Column") { column in
                    HStack(spacing: 4) {
                        if column.isPrimaryKey {
                            Image(systemName: "key.fill")
                                .font(.system(size: 9))
                                .foregroundStyle(.yellow)
                        }
                        Text(column.name).fontWeight(column.isPrimaryKey ? .semibold : .regular)
                    }
                }
                .width(min: 120, ideal: 180)
                TableColumn("Type") { Text($0.type).font(.system(size: 12, design: .monospaced)) }
                    .width(min: 90, ideal: 150)
                TableColumn("Nullable") { Text($0.nullable ? "YES" : "NO").foregroundStyle(.secondary) }
                    .width(60)
                TableColumn("Default") { column in
                    Text(column.defaultValue ?? "NULL")
                        .foregroundStyle(column.defaultValue == nil ? .tertiary : .primary)
                        .font(.system(size: 12, design: .monospaced))
                }
                .width(min: 80, ideal: 160)
                TableColumn("Extra") { Text($0.extra).foregroundStyle(.secondary) }
                    .width(min: 60, ideal: 110)
                TableColumn("Comment") { Text($0.comment).foregroundStyle(.secondary) }
            }
            .frame(minHeight: 160)

            VStack(alignment: .leading, spacing: 0) {
                Text("Indexes")
                    .font(.headline)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                Table(model.indexes) {
                    TableColumn("Name") { index in
                        HStack(spacing: 4) {
                            if index.primary {
                                Image(systemName: "key.fill").font(.system(size: 9)).foregroundStyle(.yellow)
                            }
                            Text(index.name)
                        }
                    }
                    .width(min: 120, ideal: 200)
                    TableColumn("Columns") { Text($0.columns).font(.system(size: 12, design: .monospaced)) }
                        .width(min: 100, ideal: 200)
                    TableColumn("Unique") { Text($0.unique ? "YES" : "NO").foregroundStyle(.secondary) }
                        .width(60)
                    TableColumn("Definition") { index in
                        Text(index.definition)
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }
            }
            .frame(minHeight: 120)
        }
    }
}

struct ErrorBanner: View {
    let message: String
    let onClose: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.red)
            ScrollView {
                Text(message)
                    .font(.system(size: 12, design: .monospaced))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: 80)
            .fixedSize(horizontal: false, vertical: true)
            Button(action: onClose) {
                Image(systemName: "xmark")
            }
            .buttonStyle(.borderless)
        }
        .padding(10)
        .background(Color.red.opacity(0.1))
    }
}
