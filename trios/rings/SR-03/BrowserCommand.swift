import Foundation

struct BrowserCommand: Codable, Sendable {
    let id: UUID
    let type: BrowserCommandType
    let payload: [String: String]
    let issuedAt: Date
}

enum BrowserCommandType: String, Codable, Sendable {
    case navigate, click, fill, scroll, screenshot, evaluate, closeTab, newTab
}

struct BrowserResult: Codable, Sendable {
    let commandId: UUID
    let success: Bool
    let data: String?
    let error: String?
    let completedAt: Date
}
