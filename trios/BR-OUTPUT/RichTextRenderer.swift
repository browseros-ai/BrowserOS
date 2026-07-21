import SwiftUI

struct RichMessageView: View {
    let text: String
    let isUser: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
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
        case .heading(let level, let content):
            HeadingBlockView(level: level, content: content)
        case .list(let items):
            ListBlockView(items: items)
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
        case heading(level: Int, String)
        case list([String])
    }
}

struct InlineMarkdownText: View {
    let text: String
    let isUser: Bool

    var body: some View {
        if let attributed = renderAttributed() {
            Text(attributed)
                .font(.body)
                .foregroundColor(.grokText)
        } else {
            manualMarkdown()
                .font(.body)
                .foregroundColor(.grokText)
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

// MARK: - Block Parser

private func parseBlocks(from text: String) -> [TextBlock] {
    var blocks: [TextBlock] = []
    let lines = text.components(separatedBy: .newlines)
    var i = 0

    while i < lines.count {
        let line = lines[i]

        // Code block
        if line.hasPrefix("```") {
            let lang = String(line.dropFirst(3)).trimmingCharacters(in: .whitespaces)
            var codeLines: [String] = []
            i += 1
            while i < lines.count && !lines[i].hasPrefix("```") {
                codeLines.append(lines[i])
                i += 1
            }
            blocks.append(TextBlock(type: .code(language: lang.isEmpty ? nil : lang, codeLines.joined(separator: "\n"))))
            i += 1
            continue
        }

        // Heading
        if let heading = parseHeading(line) {
            blocks.append(TextBlock(type: .heading(level: heading.level, heading.text)))
            i += 1
            continue
        }

        // List
        if isListItem(line) {
            var items: [String] = []
            while i < lines.count {
                let current = lines[i]
                if isListItem(current) {
                    items.append(stripListPrefix(current))
                    i += 1
                } else if current.trimmingCharacters(in: .whitespaces).isEmpty {
                    i += 1
                } else {
                    break
                }
            }
            if !items.isEmpty {
                blocks.append(TextBlock(type: .list(items)))
            }
            continue
        }

        // Paragraph / text block
        var paragraphLines: [String] = []
        while i < lines.count {
            let current = lines[i]
            if current.trimmingCharacters(in: .whitespaces).isEmpty {
                i += 1
                break
            }
            if current.hasPrefix("```") || parseHeading(current) != nil || isListItem(current) {
                break
            }
            paragraphLines.append(current)
            i += 1
        }
        if !paragraphLines.isEmpty {
            blocks.append(TextBlock(type: .text(paragraphLines.joined(separator: " "))))
        }
    }

    return blocks.isEmpty ? [TextBlock(type: .text(text))] : blocks
}

private func parseHeading(_ line: String) -> (level: Int, text: String)? {
    let trimmed = line.trimmingCharacters(in: .whitespaces)
    guard trimmed.hasPrefix("#") else { return nil }
    var level = 0
    for char in trimmed {
        if char == "#" { level += 1 } else { break }
    }
    guard level >= 1 && level <= 6 else { return nil }
    let text = trimmed.dropFirst(level).trimmingCharacters(in: .whitespaces)
    return (level, String(text))
}

private func isListItem(_ line: String) -> Bool {
    let trimmed = line.trimmingCharacters(in: .whitespaces)
    return trimmed.hasPrefix("- ") || trimmed.hasPrefix("* ") ||
        (trimmed.range(of: "^\\d+\\. ", options: .regularExpression) != nil)
}

private func stripListPrefix(_ line: String) -> String {
    let trimmed = line.trimmingCharacters(in: .whitespaces)
    if trimmed.hasPrefix("- ") { return String(trimmed.dropFirst(2)) }
    if trimmed.hasPrefix("* ") { return String(trimmed.dropFirst(2)) }
    if let range = trimmed.range(of: "^\\d+\\. ", options: .regularExpression) {
        return String(trimmed[range.upperBound...])
    }
    return trimmed
}

// MARK: - Block Views

struct HeadingBlockView: View {
    let level: Int
    let content: String

    private var fontSize: CGFloat {
        switch level {
        case 1: return 20
        case 2: return 18
        case 3: return 16
        default: return 14
        }
    }

    private var fontWeight: Font.Weight {
        switch level {
        case 1: return .bold
        case 2: return .semibold
        default: return .medium
        }
    }

    var body: some View {
        InlineMarkdownText(text: content, isUser: false)
            .font(.system(size: fontSize, weight: fontWeight))
            .padding(.top, level == 1 ? 4 : 2)
    }
}

struct ListBlockView: View {
    let items: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                HStack(alignment: .top, spacing: 6) {
                    Text("- ")
                        .font(.body)
                        .foregroundColor(.grokMuted)
                    InlineMarkdownText(text: item, isUser: false)
                        .font(.body)
                }
            }
        }
    }
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
                .foregroundColor(.grokMuted)

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
                    .foregroundColor(.grokMuted)
                }
                .buttonStyle(PlainButtonStyle())
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(Color.grokElevated.opacity(0.5))

            ScrollView(.horizontal, showsIndicators: true) {
                Text(code)
                    .font(.system(.body, design: .monospaced))
                    .foregroundColor(.grokText)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
            }
        }
        .background(Color.grokElevated.opacity(0.4))
        .cornerRadius(8)
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(Color.grokBorder.opacity(0.3), lineWidth: 1)
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
            HStack(spacing: 8) {
                Image(systemName: "brain.head.profile")
                    .font(.system(size: 11))
                    .foregroundColor(.grokMuted)
                Text(isExpanded ? "Thought process" : "Thinking...")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundColor(.grokMuted)

                if !isExpanded {
                    Text("\(lineCount) steps")
                        .font(.system(size: 10))
                        .foregroundColor(.grokDim)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Color.grokElevated.opacity(0.6))
                        .cornerRadius(6)
                }

                Spacer()

                Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
                    .font(.system(size: 10))
                    .foregroundColor(.grokDim)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .contentShape(Rectangle())
            .onTapGesture {
                withAnimation(.spring(response: 0.25, dampingFraction: 0.8)) {
                    isExpanded.toggle()
                }
            }

            if isExpanded {
                Divider()
                    .overlay(Color.grokBorder.opacity(0.5))
                    .padding(.horizontal, 12)

                Text(content)
                    .font(.system(size: 11))
                    .foregroundColor(.grokDim)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .background(Color.grokElevated.opacity(0.4))
        .cornerRadius(8)
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(Color.grokBorder.opacity(0.3), lineWidth: 1)
        )
    }
}
