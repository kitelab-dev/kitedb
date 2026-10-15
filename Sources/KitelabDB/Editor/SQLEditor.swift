import AppKit
import SwiftUI

/// Plain-text SQL editor with syntax colouring, auto-indent and keyword /
/// table-name completion (Esc or ⌥Esc).
struct SQLEditor: NSViewRepresentable {
    @Binding var text: String
    @Binding var selectedText: String
    var completionWords: [String] = []

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSTextView.scrollableTextView()
        let textView = scrollView.documentView as! NSTextView
        textView.delegate = context.coordinator
        textView.isRichText = false
        textView.allowsUndo = true
        textView.usesFindBar = true
        textView.isIncrementalSearchingEnabled = true
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.isAutomaticSpellingCorrectionEnabled = false
        textView.isContinuousSpellCheckingEnabled = false
        textView.isGrammarCheckingEnabled = false
        textView.smartInsertDeleteEnabled = false
        textView.font = SQLHighlighter.font
        textView.textColor = .labelColor
        textView.backgroundColor = .textBackgroundColor
        textView.textContainerInset = NSSize(width: 6, height: 8)
        textView.typingAttributes = [.font: SQLHighlighter.font, .foregroundColor: NSColor.labelColor]
        textView.string = text
        SQLHighlighter.highlight(textView)
        scrollView.borderType = .noBorder
        DispatchQueue.main.async { textView.window?.makeFirstResponder(textView) }
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        context.coordinator.parent = self
        guard let textView = scrollView.documentView as? NSTextView else { return }
        if textView.string != text {
            textView.string = text
            SQLHighlighter.highlight(textView)
        }
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: SQLEditor

        init(_ parent: SQLEditor) {
            self.parent = parent
        }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            parent.text = textView.string
            SQLHighlighter.highlight(textView)
        }

        func textViewDidChangeSelection(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            let range = textView.selectedRange()
            let selected = range.length > 0 ? (textView.string as NSString).substring(with: range) : ""
            if parent.selectedText != selected { parent.selectedText = selected }
        }

        func textView(_ textView: NSTextView, doCommandBy selector: Selector) -> Bool {
            guard selector == #selector(NSResponder.insertNewline(_:)) else { return false }
            // Keep the indentation of the current line.
            let string = textView.string as NSString
            let lineRange = string.lineRange(for: NSRange(location: textView.selectedRange().location, length: 0))
            let line = string.substring(with: lineRange)
            let indent = line.prefix { $0 == " " || $0 == "\t" }
            textView.insertText("\n" + indent, replacementRange: textView.selectedRange())
            return true
        }

        func textView(
            _ textView: NSTextView,
            completions words: [String],
            forPartialWordRange charRange: NSRange,
            indexOfSelectedItem index: UnsafeMutablePointer<Int>?
        ) -> [String] {
            let partial = (textView.string as NSString).substring(with: charRange).lowercased()
            guard !partial.isEmpty else { return [] }
            let candidates = parent.completionWords + SQLHighlighter.keywords.map(\.localizedUppercase)
            var seen = Set<String>()
            return candidates.filter { word in
                word.lowercased().hasPrefix(partial) && seen.insert(word).inserted
            }
        }
    }
}

enum SQLHighlighter {
    static let font = NSFont.monospacedSystemFont(ofSize: 13, weight: .regular)

    static let keywords: [String] = """
        select from where and or not in is null like ilike between exists as on join left right inner outer full \
        cross natural using group by order asc desc having limit offset distinct union all intersect except \
        insert into values update set delete returning create table view index unique primary key foreign \
        references alter add drop column rename to truncate cascade restrict default constraint check if \
        case when then else end begin commit rollback transaction start with recursive explain analyze show \
        databases tables describe use grant revoke database schema sequence trigger function procedure \
        true false count sum avg min max coalesce nullif cast interval now current_timestamp current_date \
        auto_increment serial bigserial int integer bigint smallint varchar char text boolean bool date \
        timestamp timestamptz time numeric decimal float double real json jsonb uuid bytea blob enum \
        lateral window over partition filter any some do nothing conflict replace ignore duplicate
        """.split(whereSeparator: \.isWhitespace).map(String.init)

    private static let keywordRegex = try! NSRegularExpression(
        pattern: "\\b(" + keywords.joined(separator: "|") + ")\\b", options: [.caseInsensitive])
    private static let numberRegex = try! NSRegularExpression(pattern: "\\b\\d+(\\.\\d+)?\\b")
    private static let quotedIdentifierRegex = try! NSRegularExpression(pattern: "`[^`]*`|\"[^\"]*\"")
    private static let stringRegex = try! NSRegularExpression(pattern: "'(?:[^'\\\\]|\\\\.|'')*'?")
    private static let commentRegex = try! NSRegularExpression(
        pattern: "--[^\\n]*|/\\*[\\s\\S]*?(\\*/|$)")

    static func highlight(_ textView: NSTextView) {
        guard let storage = textView.textStorage else { return }
        let text = storage.string
        let full = NSRange(location: 0, length: (text as NSString).length)
        // Skip colouring for huge scripts to keep typing responsive.
        guard full.length < 300_000 else { return }

        storage.beginEditing()
        storage.setAttributes([.font: font, .foregroundColor: NSColor.labelColor], range: full)
        apply(keywordRegex, color: .systemPink, in: storage, text: text, range: full)
        apply(numberRegex, color: .systemPurple, in: storage, text: text, range: full)
        apply(quotedIdentifierRegex, color: .systemTeal, in: storage, text: text, range: full)
        apply(stringRegex, color: .systemOrange, in: storage, text: text, range: full)
        apply(commentRegex, color: .secondaryLabelColor, in: storage, text: text, range: full)
        storage.endEditing()
    }

    private static func apply(
        _ regex: NSRegularExpression, color: NSColor, in storage: NSTextStorage, text: String, range: NSRange
    ) {
        for match in regex.matches(in: text, range: range) {
            storage.addAttribute(.foregroundColor, value: color, range: match.range)
        }
    }
}
