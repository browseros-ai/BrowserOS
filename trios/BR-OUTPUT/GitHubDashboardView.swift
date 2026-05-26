import SwiftUI

struct GitHubDashboardView: View {
    @StateObject private var vm = GitHubDashboardViewModel()
    @State private var selectedRepo: GitHubRepo?

    var body: some View {
        VStack(spacing: 0) {
            if selectedRepo == nil {
                repoList
            } else {
                issueList
            }
        }
        .background(Color.clear)
        .onAppear { vm.loadRepos() }
    }

    private var repoList: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Repositories")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundColor(.grokText)
                Spacer()
                if vm.isLoading {
                    ProgressView()
                        .scaleEffect(0.6)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)

            Divider().overlay(Color.grokBorder)

            if let error = vm.errorMessage {
                HStack(spacing: 6) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .font(.system(size: 10))
                        .foregroundColor(.yellow)
                    Text(error)
                        .font(.system(size: 10))
                        .foregroundColor(.grokText)
                    Spacer()
                    Button(action: { vm.errorMessage = nil }) {
                        Image(systemName: "xmark")
                            .font(.system(size: 10))
                            .foregroundColor(.grokMuted)
                    }
                    .buttonStyle(.plain)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(Color.red.opacity(0.1))
                .cornerRadius(6)
                .padding(.horizontal, 12)
                .padding(.vertical, 4)
            }

            if vm.repos.isEmpty && !vm.isLoading {
                VStack(spacing: 8) {
                    Spacer()
                    Image(systemName: "folder")
                        .font(.system(size: 32))
                        .foregroundColor(.grokDim)
                    Text("No repositories")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundColor(.grokText)
                    Text("Check GitHub connection")
                        .font(.system(size: 11))
                        .foregroundColor(.grokMuted)
                    Spacer()
                }
            } else {
                List(vm.repos) { repo in
                Button(action: { selectedRepo = repo; vm.loadIssues(for: repo) }) {
                    HStack(spacing: 8) {
                        Image(systemName: "folder.fill")
                            .foregroundColor(.grokMuted)
                            .font(.system(size: 12))
                        VStack(alignment: .leading, spacing: 2) {
                            Text(repo.name)
                                .font(.system(size: 13, design: .monospaced))
                                .foregroundColor(.grokText)
                            if let desc = repo.description {
                                Text(desc)
                                    .font(.system(size: 10))
                                    .lineLimit(1)
                                    .foregroundColor(.grokMuted)
                            }
                        }
                        Spacer()
                        if repo.open_issues_count > 0 {
                            Text("\(repo.open_issues_count)")
                                .font(.system(size: 10))
                                .padding(.horizontal, 5)
                                .padding(.vertical, 1)
                                .background(Color.red.opacity(0.2))
                                .cornerRadius(8)
                                .foregroundColor(.grokText)
                        }
                    }
                }
                .buttonStyle(.plain)
                .padding(.vertical, 2)
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            }
        }
    }

    private var issueList: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Button(action: { selectedRepo = nil }) {
                    Image(systemName: "chevron.left")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundColor(.grokMuted)
                }
                .buttonStyle(.plain)

                if let repo = selectedRepo {
                    Text(repo.name)
                        .font(.system(size: 13, weight: .semibold, design: .monospaced))
                        .foregroundColor(.grokText)
                }

                Spacer()

                Picker("", selection: $vm.issueState) {
                    Text("Open").tag("open")
                    Text("Closed").tag("closed")
                    Text("All").tag("all")
                }
                .pickerStyle(.segmented)
                .frame(width: 140)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)

            Divider().overlay(Color.grokBorder)

            if vm.filteredIssues.isEmpty {
                VStack(spacing: 8) {
                    Spacer()
                    Image(systemName: "checkmark.circle")
                        .font(.system(size: 32))
                        .foregroundColor(.grokDim)
                    Text("No issues")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundColor(.grokText)
                    Spacer()
                }
            } else {
                List(vm.filteredIssues) { issue in
                HStack(spacing: 8) {
                    Circle()
                        .fill(issue.state == "open" ? Color.green.opacity(0.8) : Color.purple.opacity(0.8))
                        .frame(width: 6, height: 6)

                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 4) {
                            Text("#\(issue.number)")
                                .font(.system(size: 10, design: .monospaced))
                                .foregroundColor(.grokMuted)
                            Text(issue.title)
                                .font(.system(size: 12))
                                .foregroundColor(.grokText)
                                .lineLimit(1)
                        }

                        HStack(spacing: 4) {
                            ForEach(issue.labels.prefix(2)) { label in
                                Text(label.name)
                                    .font(.system(size: 9))
                                    .padding(.horizontal, 4)
                                    .padding(.vertical, 1)
                                    .background(Color(hex: label.color).opacity(0.2))
                                    .cornerRadius(3)
                                    .foregroundColor(.grokMuted)
                            }
                        }
                    }

                    Spacer()
                }
                .padding(.vertical, 3)
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            }
        }
    }
}

@MainActor
class GitHubDashboardViewModel: ObservableObject {
    @Published var repos: [GitHubRepo] = []
    @Published var issues: [GitHubIssue] = []
    @Published var issueState = "all"
    @Published var isLoading = false
    @Published var errorMessage: String?

    var filteredIssues: [GitHubIssue] {
        issues.filter { issue in
            issueState == "all" || issue.state == issueState
        }
    }

    func loadRepos() {
        isLoading = true
        Task {
            do {
                repos = try await GitHubAPIClient.shared.fetchRepos()
                errorMessage = nil
            } catch {
                errorMessage = "Failed to load repos: \(error.localizedDescription)"
            }
            isLoading = false
        }
    }

    func loadIssues(for repo: GitHubRepo) {
        Task {
            do {
                issues = try await GitHubAPIClient.shared.fetchIssues(repo: repo.name, state: issueState)
            } catch {
                errorMessage = "Failed to load issues: \(error.localizedDescription)"
            }
        }
    }
}

extension Color {
    init(hex: String) {
        let scanner = Scanner(string: hex)
        var rgb: UInt64 = 0
        scanner.scanHexInt64(&rgb)
        self.init(
            red: Double((rgb >> 16) & 0xFF) / 255,
            green: Double((rgb >> 8) & 0xFF) / 255,
            blue: Double(rgb & 0xFF) / 255
        )
    }
}
