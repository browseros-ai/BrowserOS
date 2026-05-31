import Foundation

actor GitHubAPIClient {
    static let shared = GitHubAPIClient()
    let baseURL = "https://api.github.com"

    var token: String? {
        ProcessInfo.processInfo.environment["GITHUB_TOKEN"]
    }

    private func request(_ endpoint: String) throws -> URLRequest {
        guard let url = URL(string: baseURL + endpoint) else {
            throw URLError(.badURL)
        }
        var request = URLRequest(url: url)
        request.setValue("application/vnd.github.v3+json", forHTTPHeaderField: "Accept")
        if let token = token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        return request
    }

    func fetchRepos() async throws -> [GitHubRepo] {
        let (data, _) = try await URLSession.shared.data(for: request("/users/gHashTag/repos?per_page=100"))
        return try JSONDecoder().decode([GitHubRepo].self, from: data)
    }

    func fetchIssues(repo: String, state: String = "all") async throws -> [GitHubIssue] {
        let (data, _) = try await URLSession.shared.data(for: request("/repos/gHashTag/\(repo)/issues?state=\(state)&per_page=100"))
        return try JSONDecoder().decode([GitHubIssue].self, from: data)
    }

    func fetchIssueComments(repo: String, issueNumber: Int) async throws -> [GitHubComment] {
        let (data, _) = try await URLSession.shared.data(for: request("/repos/gHashTag/\(repo)/issues/\(issueNumber)/comments"))
        return try JSONDecoder().decode([GitHubComment].self, from: data)
    }

    func createIssue(repo: String, title: String, body: String, labels: [String] = []) async throws -> GitHubIssue {
        var req = try request("/repos/gHashTag/\(repo)/issues")
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let payload: [String: Any] = [
            "title": title,
            "body": body,
            "labels": labels,
        ]
        req.httpBody = try JSONSerialization.data(withJSONObject: payload)
        let (data, _) = try await URLSession.shared.data(for: req)
        return try JSONDecoder().decode(GitHubIssue.self, from: data)
    }

    func createPR(repo: String, title: String, body: String, head: String, base: String = "dev") async throws -> GitHubPullRequest {
        var req = try request("/repos/gHashTag/\(repo)/pulls")
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let payload: [String: Any] = [
            "title": title,
            "body": body,
            "head": head,
            "base": base,
        ]
        req.httpBody = try JSONSerialization.data(withJSONObject: payload)
        let (data, _) = try await URLSession.shared.data(for: req)
        return try JSONDecoder().decode(GitHubPullRequest.self, from: data)
    }

    func addComment(repo: String, issueNumber: Int, body: String) async throws -> GitHubComment {
        var req = try request("/repos/gHashTag/\(repo)/issues/\(issueNumber)/comments")
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let payload: [String: Any] = ["body": body]
        req.httpBody = try JSONSerialization.data(withJSONObject: payload)
        let (data, _) = try await URLSession.shared.data(for: req)
        return try JSONDecoder().decode(GitHubComment.self, from: data)
    }
}
