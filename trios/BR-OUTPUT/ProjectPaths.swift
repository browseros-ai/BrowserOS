import Foundation

/// Centralized path configuration for the Trios project.
/// Eliminates hardcoded strings scattered across the codebase.
enum ProjectPaths {
    /// The root directory of the Trios project.
    /// Defaults to the bundled project path or falls back to the developer path.
    static var root: String {
        // Env override is authoritative.
        if let envRoot = ProcessInfo.processInfo.environment["TRIOS_ROOT"], !envRoot.isEmpty {
            return envRoot
        }
        // Try to find the project relative to the app bundle first.
        if let bundlePath = Bundle.main.resourcePath {
            let candidate = (bundlePath as NSString).deletingLastPathComponent
            if FileManager.default.fileExists(atPath: "\(candidate)/main.swift") {
                return candidate
            }
        }
        // Fallback for development: current working directory.
        return FileManager.default.currentDirectoryPath
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

    /// MCP port from Info.plist (injected at build time via TRIOS_VARIANT)
    static var mcpPort: String {
        Bundle.main.infoDictionary?["TRIOS_MCP_PORT"] as? String ?? "9105"
    }

    /// A2A port from Info.plist
    static var a2aPort: String {
        Bundle.main.infoDictionary?["TRIOS_A2A_PORT"] as? String ?? "9200"
    }

    /// Build variant from Info.plist (prod or staging)
    static var buildVariant: String {
        Bundle.main.infoDictionary?["TRIOS_VARIANT"] as? String ?? "prod"
    }

    static var canaryMcpPort: String {
        Bundle.main.infoDictionary?["TRIOS_CANARY_MCP_PORT"] as? String ?? "9205"
    }

    static var meshPort: String {
        Bundle.main.infoDictionary?["TRIOS_MESH_PORT"] as? String ?? "9505"
    }

    static var mcpBaseURL: String { "http://127.0.0.1:\(mcpPort)" }
    static var browserOSHealthURL: String { "\(mcpBaseURL)/health" }
    static var agentHealthURL: String { "http://127.0.0.1:\(a2aPort)/health" }
    static var canaryHealthURL: String { "http://127.0.0.1:\(canaryMcpPort)/health" }
    static var meshHealthURL: String { "http://127.0.0.1:\(meshPort)/health" }
    static var meshStatusURL: String { "http://127.0.0.1:\(meshPort)/status" }

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

    // MARK: - Runtime State Paths

    static var trinityRun: String { "\(trinity)/run" }
    static var singletonLockFile: String { "\(trinityRun)/trios_singleton.lock" }
    static var singletonPIDFile: String { "\(trinityRun)/trios_singleton.pid" }
    static var bundleIdentifier: String { "com.browseros.trios" }
}
