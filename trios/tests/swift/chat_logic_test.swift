// Standalone unit tests for ChatLogic — Foundation only, no SwiftUI/Combine.
//
// Run (from trios root), consistent with the no-SPM / TDD-inside-build model:
//   swiftc tests/swift/chat_logic_test.swift BR-OUTPUT/ChatLogic.swift -o /tmp/chat_logic_test && /tmp/chat_logic_test
//
// Exits non-zero on the first failed assertion.

import Foundation

@main
enum ChatLogicTests {
    static var failures = 0

    static func check(_ cond: Bool, _ name: String) {
        if cond {
            print("ok   - \(name)")
        } else {
            print("FAIL - \(name)")
            failures += 1
        }
    }

    static func main() {
        // firstPageId — real `list_pages` text format: "0. Title (tab 12)\n   url"
        check(ChatLogic.firstPageId(in: "0. Example (tab 12)\n   https://example.com") == 0,
              "firstPageId parses leading id 0")
        check(ChatLogic.firstPageId(in: "3. Other (tab 5)\n   https://x") == 3,
              "firstPageId parses leading id 3")
        check(ChatLogic.firstPageId(in: "No pages open.") == nil,
              "firstPageId returns nil for empty listing")
        check(ChatLogic.firstPageId(in: "") == nil,
              "firstPageId returns nil for empty string")
        check(ChatLogic.firstPageId(in: "   2. Indented (tab 1)") == 2,
              "firstPageId tolerates leading whitespace")

        // isLikelyCommand — strict matching
        check(ChatLogic.isLikelyCommand("shell ls -la"), "prefix 'shell ' is a command")
        check(ChatLogic.isLikelyCommand("open https://x"), "prefix 'open ' is a command")
        check(ChatLogic.isLikelyCommand("screenshot"), "exact 'screenshot' is a command")
        check(ChatLogic.isLikelyCommand("/help"), "slash path is a command")
        check(ChatLogic.isLikelyCommand("./run.sh"), "dot-slash path is a command")
        check(!ChatLogic.isLikelyCommand("running late today"),
              "'running' does not match 'run ' prefix (no trailing space)")
        check(!ChatLogic.isLikelyCommand("what is the weather"),
              "plain question is not a command")
        check(!ChatLogic.isLikelyCommand("clicking through tabs"),
              "'clicking' does not match exact 'click' or 'click ' prefix")
        // Documents a known quirk: "swift " is a registered prefix (for swiftc),
        // so a sentence beginning "swift ..." is routed to command handling
        // (which then finds no intent and shows a usage hint — never executed).
        check(ChatLogic.isLikelyCommand("swift is a great language"),
              "'swift ' prefix is treated as a command attempt (known quirk)")

        // extractURL
        check(ChatLogic.extractURL(from: "open https://example.com/x now") == "https://example.com/x",
              "extractURL pulls the first http(s) URL")
        check(ChatLogic.extractURL(from: "no url here") == nil,
              "extractURL returns nil when absent")

        // parseIntent — tool routing
        check(ChatLogic.parseIntent("screenshot", pageId: nil)?.0 == "take_screenshot",
              "parseIntent maps 'screenshot' -> take_screenshot")
        check(ChatLogic.parseIntent("extract", pageId: nil)?.0 == "get_page_content",
              "parseIntent maps 'extract' -> get_page_content")
        check(ChatLogic.parseIntent("navigate https://x.com", pageId: nil)?.0 == "navigate_page",
              "parseIntent maps 'navigate ' -> navigate_page")
        check(ChatLogic.parseIntent("what is the weather", pageId: nil) == nil,
              "parseIntent returns nil for unrecognized input (no shell fallthrough)")

        // parseIntent — pageId threading + URL extraction
        if let nav = ChatLogic.parseIntent("open https://example.com", pageId: 7) {
            check(nav.0 == "navigate_page", "navigate tool name")
            check((nav.1["url"] as? String) == "https://example.com", "navigate url is extracted")
            check((nav.1["page"] as? Int) == 7, "navigate threads pageId")
        } else {
            check(false, "parseIntent navigate returned nil unexpectedly")
        }

        // parseIntent — SECURITY: recursive self-launch is blocked, not executed
        if let blocked = ChatLogic.parseIntent("shell open trios_app", pageId: nil) {
            check(blocked.0 == "filesystem_bash", "blocked command still routes to filesystem_bash")
            let cmd = (blocked.1["command"] as? String) ?? ""
            check(cmd.hasPrefix("echo 'Blocked"),
                  "recursive self-launch ('trios_app') is rewritten to a safe echo")
        } else {
            check(false, "parseIntent shell returned nil unexpectedly")
        }
        if let normal = ChatLogic.parseIntent("shell ls -la", pageId: nil) {
            check((normal.1["command"] as? String) == "ls -la",
                  "ordinary shell command is passed through verbatim")
        } else {
            check(false, "parseIntent shell 'ls' returned nil unexpectedly")
        }
        check(ChatLogic.parseIntent("shell    ", pageId: nil) == nil,
              "empty shell command returns nil")

        // parseIntent — every recursive-launch pattern must be blocked
        for cmd in [
            "shell ./trios_app",             // literal trios_app
            "shell open trios",              // "open trios" as a word
            "shell open trios.app",          // "open trios.app"
            "shell open trios and click",    // "open trios" followed by more words
            "shell swiftc -o trios x.swift", // "swiftc.*trios"
            "shell launchd load trios",      // "launchd.*trios"
            "shell clade-promote --boot",    // "clade-promote.*boot"
        ] {
            let parsed = ChatLogic.parseIntent(cmd, pageId: nil)
            let blocked = ((parsed?.1["command"] as? String) ?? "").hasPrefix("echo 'Blocked")
            check(blocked, "recursive-launch blocked: \(cmd)")
        }

        // Sanity: an ordinary shell command containing "trios" as a substring
        // but not as a self-launch pattern should NOT be blocked.
        if let normal = ChatLogic.parseIntent("shell echo trios is running", pageId: nil) {
            check((normal.1["command"] as? String) == "echo trios is running",
                  "innocuous 'trios' substring is not blocked")
        } else {
            check(false, "innocuous 'trios' command returned nil unexpectedly")
        }

        if failures == 0 {
            print("\nAll ChatLogic tests passed.")
            exit(0)
        } else {
            print("\n\(failures) test(s) failed.")
            exit(1)
        }
    }
}
