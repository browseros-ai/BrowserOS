import Foundation

/// Centralized path configuration for the Trios project.
/// Eliminates hardcoded strings scattered across the codebase.
enum ProjectPaths {
    /// The root directory of the Trios project.
    /// Defaults to the bundled project path or falls back to the developer path.
    static var root: String {
        // Try to find the project relative to the app bundle first
        if let bundlePath = Bundle.main.resourcePath {
            let candidate = (bundlePath as NSString).deletingLastPathComponent
            if FileManager.default.fileExists(atPath: "\(candidate)/main.swift") {
                return candidate
            }
        }
        // Fallback for development
        return "/Users/playra/BrowserOS-full/trios"
    }

    // MARK: - Subdirectories

    static var brOutput: String { "\(root)/BR-OUTPUT" }
    static var rings: String { "\(root)/rings" }
    static var claude: String { "\(root)/.claude" }
    static var trinity: String { "\(root)/.trinity" }

    // MARK: - Key Files

    static var mainSwift: String { "\(root)/main.swift" }
    static var buildScript: String { "\(root)/build.sh" }
    static var triosBinary: String { "\(root)/trios_app" }
    static var appBundle: String { "\(root)/trios.app" }
    static var logoPNG: String { "\(root)/logo.png" }
    static var logoSVG: String { "\(root)/logo.svg" }

    // MARK: - BrowserOS Agent Server

    static var browserOSAgentRoot: String { "\(root)/../packages/browseros-agent" }
    static var browserOSHealthURL: String { "http://127.0.0.1:9105/health" }
    static var agentHealthURL: String { "http://127.0.0.1:9200/health" }

    // MARK: - Trinity State

    static var trinityState: String { "\(trinity)/state/last_wake.json" }
    static var trinityLog: String { "\(trinity)/cron.log" }
    static var trinityEventLog: String { "\(trinity)/event_log.jsonl" }

    // MARK: - Helpers

    static func rings(_ subdir: String) -> String {
        "\(root)/rings/\(subdir)"
    }

    static func brOutput(_ file: String) -> String {
        "\(root)/BR-OUTPUT/\(file)"
    }

    static func claude(_ subpath: String) -> String {
        "\(root)/.claude/\(subpath)"
    }
}
