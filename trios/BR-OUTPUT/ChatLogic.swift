import Foundation

/// Pure, framework-free chat parsing helpers extracted from
/// `BrowserOSChatViewModel` so they can be unit-tested standalone with swiftc
/// (Foundation only, no SwiftUI/Combine). See tests/swift/chat_logic_test.swift.
enum ChatLogic {

    /// Extract the first page id from a `list_pages` text listing. Each page
    /// entry starts with `"<id>. "`; returns the id of the first such entry.
    /// (The MCP `list_pages` tool returns human-readable text, not JSON.)
    static func firstPageId(in text: String) -> Int? {
        for line in text.split(separator: "\n") {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            guard let dotIndex = trimmed.firstIndex(of: ".") else { continue }
            if let id = Int(trimmed[..<dotIndex]) {
                return id
            }
        }
        return nil
    }

    /// Prefixes (each ending in a space) that mark an explicit command. The
    /// trailing space prevents matching innocent words like "running".
    static let explicitPrefixes = [
        "shell ", "run ", "exec ", "navigate ", "click ", "screenshot ", "extract ",
        "open ", "go to ", "browse ", "cat ", "ls ", "cd ", "mkdir ", "rm ",
        "git ", "curl ", "wget ", "npm ", "bun ", "node ", "python ", "swift ",
    ]

    /// Single-word commands that must match exactly (not as a substring).
    static let exactCommands = ["click", "screenshot", "extract", "pwd"]

    /// Whether `text` should be routed to command execution rather than the LLM.
    /// Strict matching only: explicit prefix, exact single-word, or a slash path.
    static func isLikelyCommand(_ text: String) -> Bool {
        let lower = text.lowercased().trimmingCharacters(in: .whitespacesAndNewlines)
        let isPrefixMatch = explicitPrefixes.contains { lower.hasPrefix($0) }
        let isExactMatch = exactCommands.contains { lower == $0 }
        let isSlashCommand = lower.hasPrefix("/") || lower.hasPrefix("./")
        return isPrefixMatch || isExactMatch || isSlashCommand
    }
}
