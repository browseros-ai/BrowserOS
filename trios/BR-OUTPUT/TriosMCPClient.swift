import Foundation
import SwiftUI

/// Actor that communicates with BrowserOS MCP server (port 9105) for reverse integration.
/// Trios (SwiftUI) -> TriosMCPClient -> HTTP JSON-RPC -> MCP Server -> BrowserOS Agent
@MainActor
final class TriosMCPClient: ObservableObject {
    private let serverURL: URL
    private let session: URLSession

    @Published var isConnected = false
    @Published var lastError: String?
    @Published var browserState = BrowserState()

    init(serverURL: URL = URL(string: ProjectPaths.mcpBaseURL) ?? URL(fileURLWithPath: "/dev/null")) {
        self.serverURL = serverURL
        let config = URLSessionConfiguration.default
        config.timeoutIntervalForRequest = 30
        config.timeoutIntervalForResource = 300
        self.session = URLSession(configuration: config)
    }

    // MARK: - Health

    func checkHealth() async -> Bool {
        guard let url = URL(string: "\(serverURL.absoluteString)/health") else { return false }
        do {
            let (_, response) = try await session.data(from: url)
            let ok = (response as? HTTPURLResponse)?.statusCode == 200
            isConnected = ok
            return ok
        } catch {
            lastError = error.localizedDescription
            isConnected = false
            return false
        }
    }

    // MARK: - Generic Tool Call

    func callTool(name: String, arguments: [String: Any]) async throws -> MCPResponse {
        let url = serverURL.appendingPathComponent("mcp")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")

        let body: [String: Any] = [
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": [
                "name": name,
                "arguments": arguments
            ],
            "id": Int.random(in: 1...999999)
        ]

        request.httpBody = try JSONSerialization.data(withJSONObject: body)

        let (data, response) = try await session.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse, httpResponse.statusCode == 200 else {
            throw MCPError.serverError
        }

        let decoded = try JSONDecoder().decode(MCPResponse.self, from: data)
        return decoded
    }

    // MARK: - Filesystem Tools

    func readFile(path: String, limit: Int? = nil, offset: Int? = nil) async throws -> String {
        var args: [String: Any] = ["path": path]
        if let limit = limit { args["limit"] = limit }
        if let offset = offset { args["offset"] = offset }
        let response = try await callTool(name: "fs_read", arguments: args)
        return response.textContent ?? ""
    }

    func writeFile(path: String, content: String) async throws {
        let args: [String: Any] = ["path": path, "content": content]
        _ = try await callTool(name: "fs_write", arguments: args)
    }

    func listDirectory(path: String) async throws -> String {
        let response = try await callTool(name: "fs_list", arguments: ["path": path])
        return response.textContent ?? ""
    }

    // MARK: - Shell (working tool: filesystem_bash)

    func shell(command: String, description: String = "trios shell") async throws -> String {
        let args: [String: Any] = [
            "command": command,
            "description": description
        ]
        let response = try await callTool(name: "filesystem_bash", arguments: args)
        return response.textContent ?? ""
    }

    // MARK: - Browser Tools

    func navigate(to urlString: String, pageId: Int? = nil) async throws {
        var args: [String: Any] = ["url": urlString]
        if let pageId = pageId { args["page"] = pageId }
        _ = try await callTool(name: "navigate_page", arguments: args)
        browserState.currentURL = urlString
    }

    func click(element: String, pageId: Int? = nil) async throws {
        var args: [String: Any] = ["element": element]
        if let pageId = pageId { args["page"] = pageId }
        _ = try await callTool(name: "click", arguments: args)
    }

    func type(text: String, into element: String, pageId: Int? = nil) async throws {
        var args: [String: Any] = [
            "element": element,
            "text": text
        ]
        if let pageId = pageId { args["page"] = pageId }
        _ = try await callTool(name: "type_at", arguments: args)
    }

    func takeScreenshot(pageId: Int? = nil) async throws -> Data {
        var args: [String: Any] = [:]
        if let pageId = pageId { args["page"] = pageId }
        let response = try await callTool(name: "take_screenshot", arguments: args)
        // Screenshot returns base64 or URL; parse accordingly
        guard let text = response.textContent else {
            throw MCPError.invalidResponse
        }
        // If base64 encoded image data
        if let data = Data(base64Encoded: text) {
            return data
        }
        throw MCPError.invalidResponse
    }

    func getPageContent(pageId: Int? = nil) async throws -> String {
        var args: [String: Any] = [:]
        if let pageId = pageId { args["page"] = pageId }
        let response = try await callTool(name: "get_page_content", arguments: args)
        return response.textContent ?? ""
    }

    func scroll(direction: String, amount: Int = 3, pageId: Int? = nil) async throws {
        var args: [String: Any] = [
            "direction": direction,
            "amount": amount
        ]
        if let pageId = pageId { args["page"] = pageId }
        _ = try await callTool(name: "scroll", arguments: args)
    }

    func listPages() async throws -> String {
        let response = try await callTool(name: "list_pages", arguments: [:])
        return response.textContent ?? ""
    }

    // MARK: - Cleanup

    func disconnect() {
        session.invalidateAndCancel()
    }
}

// MARK: - Models

struct BrowserState {
    var currentURL: String?
    var pageTitle: String?
    var isLoading = false
    var tabs: [BrowserTab] = []
}

struct BrowserTab: Identifiable {
    let id = UUID()
    let url: String
    let title: String
    let favicon: Data?
}

struct MCPResponse: Codable {
    let jsonrpc: String
    let result: MCPResult?
    let error: MCPErrorDetail?
    let id: Int

    var textContent: String? {
        result?.content.first?.text
    }

    var isError: Bool {
        error != nil || (result?.isError ?? false)
    }
}

struct MCPResult: Codable {
    let content: [MCPContent]
    let isError: Bool?
}

struct MCPContent: Codable {
    let type: String
    let text: String?
}

struct MCPErrorDetail: Codable {
    let code: Int
    let message: String
}

enum MCPError: Error, LocalizedError {
    case invalidURL
    case serverError
    case noData
    case invalidResponse
    case toolNotFound

    var errorDescription: String? {
        switch self {
        case .invalidURL: return "Invalid server URL"
        case .serverError: return "HTTP request failed"
        case .noData: return "No data received"
        case .invalidResponse: return "Invalid server response"
        case .toolNotFound: return "MCP tool not found"
        }
    }
}
