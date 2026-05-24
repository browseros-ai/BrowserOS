import SwiftUI

struct RichMessageView: View {
    let text: String
    let isUser: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(blocks, id: \.id) { block in
                blockView(block)
            }
        }
    }

    private var blocks: [TextBlock] {
        parseBlocks(from: text)
    }

    @ViewBuilder
    private func blockView(_ block: TextBlock) -> some View {
        switch block.type {
        case .code(let language, let code):
            CodeBlockView(language: language, code: code)
        case .text(let markdown):
            InlineMarkdownText(text: markdown, isUser: isUser)
        }
    }
}

private struct TextBlock: Identifiable {
    let id = UUID()
    let type: BlockType
    enum BlockType {
        case text(String)
        case code(language: String?, String)
    }
}

struct InlineMarkdownText: View {
    let text: String
    let isUser: Bool

    var body: some View {
        if let attributed = renderAttributed() {
            Text(attributed)
                .font(.body)
                .foregroundColor(isUser ? .primary : .primary)
        } else {
            manualMarkdown()
                .font(.body)
                .foregroundColor(isUser ? .primary : .primary)
        }
    }

    private func renderAttributed() -> AttributedString? {
        let options = AttributedString.MarkdownParsingOptions(
            interpretedSyntax: .inlineOnlyPreservingWhitespace,
            failurePolicy: .returnPartiallyParsedIfPossible
        )
        return try? AttributedString(markdown: text, options: options)
    }

    @ViewBuilder
    private func manualMarkdown() -> some View {
        let segments = parseInline(text)
        if segments.count == 1, case .plain(let s) = segments.first {
            Text(s)
        } else {
            segments.reduce(Text("")) { acc, seg in
                switch seg {
                case .plain(let s):
                    return acc + Text(s)
                case .bold(let s):
                    return acc + Text(s).fontWeight(.bold)
                case .italic(let s):
                    return acc + Text(s).italic()
                case .code(let s):
                    return acc + Text(s).font(.system(.body, design: .monospaced))
                }
            }
        }
    }
}

private enum InlineSegment {
    case plain(String)
    case bold(String)
    case italic(String)
    case code(String)
}

private func parseInline(_ text: String) -> [InlineSegment] {
    var segments: [InlineSegment] = []
    let pattern = "(\\*\\*(.+?)\\*\\*|_(.+?)_|`(.+?)`)"
    guard let regex = try? NSRegularExpression(pattern: pattern, options: []) else {
        return [.plain(text)]
    }
    let nsRange = NSRange(text.startIndex..., in: text)
    let matches = regex.matches(in: text, options: [], range: nsRange)
    var last = text.startIndex
    for match in matches {
        let range = Range(match.range, in: text)!
        if last < range.lowerBound {
            segments.append(.plain(String(text[last..<range.lowerBound])))
        }
        let full = String(text[range])
        if full.hasPrefix("**"), let inner = Range(match.range(at: 2), in: text) {
            segments.append(.bold(String(text[inner])))
        } else if full.hasPrefix("_"), let inner = Range(match.range(at: 3), in: text) {
            segments.append(.italic(String(text[inner])))
        } else if full.hasPrefix("`"), let inner = Range(match.range(at: 4), in: text) {
            segments.append(.code(String(text[inner])))
        } else {
            segments.append(.plain(full))
        }
        last = range.upperBound
    }
    if last < text.endIndex {
        segments.append(.plain(String(text[last...])))
    }
    return segments.isEmpty ? [.plain(text)] : segments
}

private func parseBlocks(from text: String) -> [TextBlock] {
    var blocks: [TextBlock] = []
    let pattern = "```(\\w*)\\n(.*?)\\n```"
    let regex = try? NSRegularExpression(pattern: pattern, options: [.dotMatchesLineSeparators])
    let nsRange = NSRange(text.startIndex..., in: text)
    var lastIndex = text.startIndex

    if let matches = regex?.matches(in: text, options: [], range: nsRange) {
        for match in matches {
            let matchRange = Range(match.range, in: text)!
            let prefix = String(text[lastIndex..<matchRange.lowerBound])
            if !prefix.isEmpty {
                blocks.append(TextBlock(type: .text(prefix)))
            }
            let langRange = match.range(at: 1)
            let lang = langRange.location != NSNotFound ? String(text[Range(langRange, in: text)!]) : nil
            let codeRange = match.range(at: 2)
            let code = String(text[Range(codeRange, in: text)!])
            blocks.append(TextBlock(type: .code(language: lang?.isEmpty ?? true ? nil : lang, code)))
            lastIndex = matchRange.upperBound
        }
    }

    let suffix = String(text[lastIndex...])
    if !suffix.isEmpty {
        blocks.append(TextBlock(type: .text(suffix)))
    }

    return blocks.isEmpty ? [TextBlock(type: .text(text))] : blocks
}

struct CodeBlockView: View {
    let language: String?
    let code: String
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                HStack(spacing: 4) {
                    Image(systemName: "chevron.left.forwardslash.chevron.right")
                        .font(.caption)
                    Text(language?.uppercased() ?? "CODE")
                        .font(.caption2)
                        .fontWeight(.semibold)
                }
                .foregroundColor(.secondary)

                Spacer()

                Button(action: {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(code, forType: .string)
                    copied = true
                    DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                        copied = false
                    }
                }) {
                    HStack(spacing: 4) {
                        Image(systemName: copied ? "checkmark" : "doc.on.doc")
                            .font(.caption)
                        Text(copied ? "Copied" : "Copy")
                            .font(.caption2)
                    }
                    .foregroundColor(.secondary)
                }
                .buttonStyle(PlainButtonStyle())
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(Color(NSColor.controlBackgroundColor).opacity(0.8))

            ScrollView(.horizontal, showsIndicators: true) {
                Text(code)
                    .font(.system(.body, design: .monospaced))
                    .foregroundColor(.primary)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
            }
        }
        .background(Color(NSColor.controlBackgroundColor))
        .cornerRadius(8)
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(Color.gray.opacity(0.3), lineWidth: 1)
        )
    }
}

struct ReasoningCollapsibleView: View {
    let content: String
    @State private var isExpanded = false

    private var lineCount: Int {
        content.components(separatedBy: .newlines).filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }.count
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button(action: { withAnimation(.spring(response: 0.25, dampingFraction: 0.8)) { isExpanded.toggle() } }) {
                HStack(spacing: 8) {
                    Image(systemName: "brain.head.profile")
                        .font(.caption)
                        .foregroundColor(.secondary)
                    Text(isExpanded ? "Thought process" : "Thinking...")
                        .font(.caption)
                        .fontWeight(.semibold)
                        .foregroundColor(.secondary)

                    if !isExpanded {
                        Text("\(lineCount) steps")
                            .font(.caption2)
                            .foregroundColor(.gray)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(Color(NSColor.controlBackgroundColor).opacity(0.8))
                            .cornerRadius(6)
                    }

                    Spacer()

                    Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
                        .font(.caption2)
                        .foregroundColor(.gray)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .contentShape(Rectangle())
            }
            .buttonStyle(PlainButtonStyle())

            if isExpanded {
                Divider()
                    .background(Color.gray.opacity(0.3))
                    .padding(.horizontal, 12)

                Text(content)
                    .font(.caption)
                    .foregroundColor(.gray)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .background(Color(NSColor.controlBackgroundColor).opacity(0.8))
        .cornerRadius(8)
    }
}
