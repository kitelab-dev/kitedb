import AppKit
import SwiftUI

enum GridRowState {
    case normal, inserted, deleted
}

struct GridCell {
    var value: String?
    /// Shown greyed out instead of `NULL` when the value is nil (e.g. DEFAULT).
    var placeholder: String?
    var edited = false
}

/// A spreadsheet-like grid on top of a view-based NSTableView. Rows are
/// pulled lazily through `cell`, so it stays fast for large results.
struct DataGrid: NSViewRepresentable {
    var columns: [String]
    var rowCount: Int
    /// Bump to make the grid reload its visible rows.
    var version: Int
    var cell: (Int, Int) -> GridCell
    var rowState: (Int) -> GridRowState = { _ in .normal }
    var editable = false
    var sortColumn: String?
    var sortAscending = true
    @Binding var selection: IndexSet
    var onEdit: ((Int, Int, String?) -> Void)?
    var onSort: ((Int) -> Void)?
    var onDelete: ((IndexSet) -> Void)?

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let tableView = GridTableView()
        tableView.coordinator = context.coordinator
        tableView.style = .plain
        tableView.usesAlternatingRowBackgroundColors = true
        tableView.gridStyleMask = [.solidVerticalGridLineMask]
        tableView.intercellSpacing = NSSize(width: 0, height: 0)
        tableView.rowHeight = 22
        tableView.allowsMultipleSelection = true
        tableView.allowsColumnReordering = false
        tableView.columnAutoresizingStyle = .noColumnAutoresizing
        tableView.dataSource = context.coordinator
        tableView.delegate = context.coordinator
        tableView.target = context.coordinator
        tableView.doubleAction = #selector(Coordinator.doubleClicked(_:))
        let menu = NSMenu()
        menu.delegate = context.coordinator
        tableView.menu = menu

        let scrollView = NSScrollView()
        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder
        context.coordinator.tableView = tableView
        context.coordinator.rebuildColumns()
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        let coordinator = context.coordinator
        let previous = coordinator.parent
        coordinator.parent = self
        guard let tableView = coordinator.tableView else { return }

        if previous.columns != columns || coordinator.builtColumns != columns.count {
            coordinator.rebuildColumns()
            tableView.reloadData()
        } else if previous.version != version || previous.rowCount != rowCount || previous.editable != editable {
            tableView.reloadData()
        }
        coordinator.updateSortIndicators()

        if !coordinator.selectionPending && tableView.selectedRowIndexes != selection {
            let valid = selection.filteredIndexSet { $0 < rowCount }
            tableView.selectRowIndexes(valid, byExtendingSelection: false)
            if let last = valid.last { tableView.scrollRowToVisible(last) }
        }
    }

    // MARK: - Coordinator

    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate, NSTextFieldDelegate, NSMenuDelegate {
        var parent: DataGrid
        weak var tableView: GridTableView?
        var builtColumns = 0
        /// True between a user selection change and it reaching the binding.
        var selectionPending = false
        private var editingCancelled = false

        private static let font = NSFont.systemFont(ofSize: 12)
        private static let placeholderFont = NSFont.systemFont(ofSize: 12).withTraits(.italic)

        init(_ parent: DataGrid) {
            self.parent = parent
        }

        func rebuildColumns() {
            guard let tableView else { return }
            for column in tableView.tableColumns.reversed() {
                tableView.removeTableColumn(column)
            }
            for (index, name) in parent.columns.enumerated() {
                let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("\(index)"))
                column.title = name
                column.headerCell.lineBreakMode = .byTruncatingTail
                column.width = idealWidth(column: index, title: name)
                column.minWidth = 40
                column.maxWidth = 2000
                tableView.addTableColumn(column)
            }
            builtColumns = parent.columns.count
        }

        private func idealWidth(column: Int, title: String) -> CGFloat {
            let attributes: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: 12, weight: .semibold)]
            var width = (title as NSString).size(withAttributes: attributes).width + 28
            for row in 0..<min(parent.rowCount, 60) {
                guard let value = parent.cell(row, column).value else { continue }
                let text = Self.displayText(value) as NSString
                width = max(width, text.size(withAttributes: [.font: Self.font]).width + 16)
            }
            return min(max(width, 64), 360)
        }

        func updateSortIndicators() {
            guard let tableView else { return }
            for (index, column) in tableView.tableColumns.enumerated() {
                let sorted = parent.columns.indices.contains(index) && parent.columns[index] == parent.sortColumn
                let image = sorted
                    ? NSImage(named: parent.sortAscending ? "NSAscendingSortIndicator" : "NSDescendingSortIndicator")
                    : nil
                if tableView.indicatorImage(in: column) !== image {
                    tableView.setIndicatorImage(image, in: column)
                }
            }
        }

        static func displayText(_ value: String) -> String {
            let prefix = value.count > 400 ? String(value.prefix(400)) + "…" : value
            return prefix.replacingOccurrences(of: "\r\n", with: " ").replacingOccurrences(of: "\n", with: " ")
        }

        // MARK: Data source & delegate

        func numberOfRows(in tableView: NSTableView) -> Int { parent.rowCount }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            guard let tableColumn, let column = Int(tableColumn.identifier.rawValue),
                  row < parent.rowCount, column < parent.columns.count else { return nil }
            let identifier = NSUserInterfaceItemIdentifier("GridCell")
            let view = tableView.makeView(withIdentifier: identifier, owner: nil) as? GridCellView
                ?? GridCellView(identifier: identifier, delegate: self)
            configure(view, row: row, column: column)
            return view
        }

        private func configure(_ view: GridCellView, row: Int, column: Int) {
            let cell = parent.cell(row, column)
            let field = view.field
            field.isEditable = false
            if let value = cell.value {
                field.stringValue = Self.displayText(value)
                field.font = Self.font
                field.textColor = parent.rowState(row) == .deleted ? .secondaryLabelColor : .labelColor
            } else {
                field.stringValue = cell.placeholder ?? "NULL"
                field.font = Self.placeholderFont
                field.textColor = .tertiaryLabelColor
            }
            view.highlight = cell.edited ? NSColor.systemYellow.withAlphaComponent(0.28) : nil
        }

        func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
            let view = GridRowView()
            switch parent.rowState(row) {
            case .normal: view.tint = nil
            case .inserted: view.tint = NSColor.systemGreen.withAlphaComponent(0.16)
            case .deleted: view.tint = NSColor.systemRed.withAlphaComponent(0.2)
            }
            return view
        }

        func tableView(_ tableView: NSTableView, didClick tableColumn: NSTableColumn) {
            guard let index = Int(tableColumn.identifier.rawValue) else { return }
            parent.onSort?(index)
        }

        func tableViewSelectionDidChange(_ notification: Notification) {
            guard let tableView else { return }
            let selected = tableView.selectedRowIndexes
            selectionPending = true
            DispatchQueue.main.async {
                self.selectionPending = false
                if self.parent.selection != selected { self.parent.selection = selected }
            }
        }

        // MARK: Editing

        @objc func doubleClicked(_ sender: Any?) {
            guard let tableView, tableView.clickedRow >= 0, tableView.clickedColumn >= 0 else { return }
            beginEditing(row: tableView.clickedRow, column: tableView.clickedColumn)
        }

        func canEdit(row: Int) -> Bool {
            parent.editable && parent.onEdit != nil && row < parent.rowCount && parent.rowState(row) != .deleted
        }

        func beginEditing(row: Int, column: Int) {
            guard let tableView, canEdit(row: row), column < parent.columns.count,
                  let view = tableView.view(atColumn: column, row: row, makeIfNecessary: true) as? GridCellView
            else { return }
            tableView.scrollColumnToVisible(column)
            editingCancelled = false
            let field = view.field
            field.isEditable = true
            field.font = Self.font
            field.textColor = .labelColor
            field.stringValue = parent.cell(row, column).value ?? ""
            tableView.window?.makeFirstResponder(field)
            field.currentEditor()?.selectAll(nil)
        }

        func controlTextDidEndEditing(_ notification: Notification) {
            guard let tableView, let field = notification.object as? NSTextField else { return }
            let row = tableView.row(for: field)
            let column = tableView.column(for: field)
            field.isEditable = false
            guard row >= 0, column >= 0, row < parent.rowCount else { return }

            if !editingCancelled {
                let old = parent.cell(row, column).value
                let new = field.stringValue
                // Leaving a NULL cell empty keeps it NULL.
                if new != (old ?? "") || (old == nil && !new.isEmpty) {
                    parent.onEdit?(row, column, new)
                }
            }
            editingCancelled = false
            tableView.reloadData(forRowIndexes: IndexSet(integer: row), columnIndexes: IndexSet(integer: column))

            let movement = notification.userInfo?["NSTextMovement"] as? Int
            if movement == NSTextMovement.tab.rawValue, column + 1 < parent.columns.count {
                DispatchQueue.main.async { self.beginEditing(row: row, column: column + 1) }
            } else if movement == NSTextMovement.backtab.rawValue, column > 0 {
                DispatchQueue.main.async { self.beginEditing(row: row, column: column - 1) }
            } else {
                tableView.window?.makeFirstResponder(tableView)
            }
        }

        func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
            if selector == #selector(NSResponder.cancelOperation(_:)) {
                editingCancelled = true
                control.window?.makeFirstResponder(tableView)
                return true
            }
            if selector == #selector(NSResponder.insertNewline(_:)) && NSEvent.modifierFlags.contains(.option) {
                textView.insertNewlineIgnoringFieldEditor(nil)
                return true
            }
            return false
        }

        // MARK: Copy & context menu

        func targetRows() -> IndexSet {
            guard let tableView else { return [] }
            let clicked = tableView.clickedRow
            if clicked >= 0 && !tableView.selectedRowIndexes.contains(clicked) {
                tableView.selectRowIndexes(IndexSet(integer: clicked), byExtendingSelection: false)
            }
            return tableView.selectedRowIndexes
        }

        func copyRows(_ rows: IndexSet) {
            guard !rows.isEmpty else { return }
            let lines = rows.map { row in
                parent.columns.indices.map { parent.cell(row, $0).value ?? "NULL" }.joined(separator: "\t")
            }
            setPasteboard(lines.joined(separator: "\n"))
        }

        private func setPasteboard(_ text: String) {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
        }

        func menuNeedsUpdate(_ menu: NSMenu) {
            menu.removeAllItems()
            guard let tableView, tableView.clickedRow >= 0 else { return }
            let row = tableView.clickedRow
            let column = tableView.clickedColumn
            let rows = targetRows()
            let plural = rows.count > 1 ? "s" : ""

            if column >= 0 {
                menu.addItem(item("Copy Value", #selector(copyValue(_:)), tag: column))
            }
            menu.addItem(item("Copy Row\(plural)", #selector(copySelectedRows(_:))))
            if column >= 0 {
                menu.addItem(item("Copy Column Name", #selector(copyColumnName(_:)), tag: column))
            }
            if canEdit(row: row), column >= 0 {
                menu.addItem(.separator())
                menu.addItem(item("Edit Value", #selector(editValue(_:)), tag: column))
                menu.addItem(item("Set NULL", #selector(setNull(_:)), tag: column))
                menu.addItem(item("Set Empty String", #selector(setEmpty(_:)), tag: column))
            }
            if parent.editable, parent.onDelete != nil {
                menu.addItem(.separator())
                menu.addItem(item("Delete Row\(plural)", #selector(deleteRows(_:))))
            }
        }

        private func item(_ title: String, _ action: Selector, tag: Int = 0) -> NSMenuItem {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
            item.target = self
            item.tag = tag
            return item
        }

        @objc private func copyValue(_ sender: NSMenuItem) {
            guard let row = tableView?.clickedRow, row >= 0 else { return }
            setPasteboard(parent.cell(row, sender.tag).value ?? "NULL")
        }

        @objc private func copySelectedRows(_ sender: Any?) {
            copyRows(tableView?.selectedRowIndexes ?? [])
        }

        @objc private func copyColumnName(_ sender: NSMenuItem) {
            setPasteboard(parent.columns[sender.tag])
        }

        @objc private func editValue(_ sender: NSMenuItem) {
            guard let row = tableView?.clickedRow, row >= 0 else { return }
            beginEditing(row: row, column: sender.tag)
        }

        @objc private func setNull(_ sender: NSMenuItem) {
            for row in tableView?.selectedRowIndexes ?? [] where canEdit(row: row) {
                parent.onEdit?(row, sender.tag, nil)
            }
        }

        @objc private func setEmpty(_ sender: NSMenuItem) {
            for row in tableView?.selectedRowIndexes ?? [] where canEdit(row: row) {
                parent.onEdit?(row, sender.tag, "")
            }
        }

        @objc private func deleteRows(_ sender: Any?) {
            guard let rows = tableView?.selectedRowIndexes, !rows.isEmpty else { return }
            parent.onDelete?(rows)
        }

        func deleteSelectedRows() {
            guard parent.editable else { return }
            deleteRows(nil)
        }
    }
}

final class GridTableView: NSTableView {
    weak var coordinator: DataGrid.Coordinator?

    override func keyDown(with event: NSEvent) {
        switch event.keyCode {
        case 51, 117: // delete, forward delete
            coordinator?.deleteSelectedRows()
        case 36, 76: // return, enter
            if selectedRow >= 0 {
                let column = max(0, columnIndexes(in: visibleRect).first ?? 0)
                coordinator?.beginEditing(row: selectedRow, column: column)
            }
        default:
            super.keyDown(with: event)
        }
    }

    @objc func copy(_ sender: Any?) {
        coordinator?.copyRows(selectedRowIndexes)
    }

    override func validateUserInterfaceItem(_ item: NSValidatedUserInterfaceItem) -> Bool {
        if item.action == #selector(copy(_:)) { return !selectedRowIndexes.isEmpty }
        return super.validateUserInterfaceItem(item)
    }
}

final class GridCellView: NSTableCellView {
    let field = NSTextField()

    var highlight: NSColor? {
        didSet { layer?.backgroundColor = highlight?.cgColor }
    }

    init(identifier: NSUserInterfaceItemIdentifier, delegate: NSTextFieldDelegate) {
        super.init(frame: .zero)
        self.identifier = identifier
        wantsLayer = true
        field.isBordered = false
        field.drawsBackground = false
        field.isEditable = false
        field.isSelectable = false
        field.lineBreakMode = .byTruncatingTail
        field.cell?.usesSingleLineMode = true
        field.cell?.truncatesLastVisibleLine = true
        field.delegate = delegate
        field.translatesAutoresizingMaskIntoConstraints = false
        addSubview(field)
        textField = field
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            field.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -4),
            field.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
}

final class GridRowView: NSTableRowView {
    var tint: NSColor?

    override func drawBackground(in dirtyRect: NSRect) {
        super.drawBackground(in: dirtyRect)
        if let tint {
            tint.setFill()
            bounds.fill()
        }
    }
}

extension NSFont {
    func withTraits(_ traits: NSFontDescriptor.SymbolicTraits) -> NSFont {
        let descriptor = fontDescriptor.withSymbolicTraits(traits)
        return NSFont(descriptor: descriptor, size: pointSize) ?? self
    }
}
