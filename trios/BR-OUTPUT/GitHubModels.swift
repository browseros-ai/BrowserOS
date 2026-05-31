import Foundation

struct GitHubRepo: Codable, Identifiable {
    let id: Int
    let name: String
    let description: String?
    let open_issues_count: Int
    let updated_at: String
    let html_url: String
}

struct GitHubIssue: Codable, Identifiable {
    let id: Int
    let number: Int
    let title: String
    let state: String
    let created_at: String
    let html_url: String
    let labels: [GitHubLabel]
    let body: String?
}

struct GitHubLabel: Codable, Identifiable {
    let id: Int?
    let name: String
    let color: String
}

struct GitHubComment: Codable, Identifiable {
    let id: Int
    let user: GitHubUser
    let body: String
    let created_at: String
}

struct GitHubUser: Codable {
    let login: String
    let avatar_url: String?
}

struct GitHubPullRequest: Codable, Identifiable {
    let id: Int
    let number: Int
    let title: String
    let state: String
    let html_url: String
    let head: GitHubBranchRef?
    let base: GitHubBranchRef?
}

struct GitHubBranchRef: Codable {
    let ref: String
    let sha: String
    let repo: GitHubRepo?
}
