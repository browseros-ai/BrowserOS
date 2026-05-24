import Foundation

actor GitHubAPIClient {
    static let shared = GitHubAPIClient()
    let baseURL = "https://api.github.com"

    var token: String? {
        ProcessInfo.processInfo.environment["GITHUB_TOKEN"]
    }

    private func request(_ endpoint: String) -> URLRequest {
        var request = URLRequest(url: URL(string: baseURL + endpoint)!)
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
}
