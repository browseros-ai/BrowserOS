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

        if failures == 0 {
            print("\nAll ChatLogic tests passed.")
            exit(0)
        } else {
            print("\n\(failures) test(s) failed.")
            exit(1)
        }
    }
}
