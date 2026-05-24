import Foundation

struct VirtualBranch: Identifiable, Codable {
    let id: String
    let name: String
    let isApplied: Bool
    let isConflicted: Bool
    let files: Int
    let commitCount: Int
    let upstream: String?
}

struct ButStatus: Codable {
    let branches: [VirtualBranch]
    let head: String
}
