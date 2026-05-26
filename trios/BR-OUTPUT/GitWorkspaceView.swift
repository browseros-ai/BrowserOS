import SwiftUI

enum GitSubTab: String, CaseIterable {
    case repos = "Repos"
    case issues = "Issues"
    case branches = "Branches"
}

struct GitWorkspaceView: View {
    @State private var selectedSubTab: GitSubTab = .repos

    var body: some View {
        VStack(spacing: 0) {
            subTabSwitcher
            Divider().overlay(Color.grokBorder)
            subTabContent
        }
        .background(Color.clear)
    }

    private var subTabSwitcher: some View {
        HStack(spacing: 4) {
            ForEach(GitSubTab.allCases, id: \.self) { tab in
                subTabButton(for: tab)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private func subTabButton(for tab: GitSubTab) -> some View {
        let isSelected = selectedSubTab == tab
        return Button(action: { selectedSubTab = tab }) {
            Text(tab.rawValue)
                .font(.system(size: 12, weight: isSelected ? .semibold : .regular))
                .foregroundColor(isSelected ? .grokText : .grokMuted)
                .padding(.horizontal, 14)
                .padding(.vertical, 6)
                .background(
                    isSelected
                        ? Color.grokElevated.opacity(0.6)
                        : Color.clear
                )
                .cornerRadius(6)
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder
    private var subTabContent: some View {
        switch selectedSubTab {
        case .repos:
            GitHubDashboardView()
        case .issues:
            GitHubIssuesView()
        case .branches:
            GitButlerPanelView()
        }
    }
}

// Placeholder until GitHubIssuesView is extracted
struct GitHubIssuesView: View {
    var body: some View {
        VStack {
            Spacer()
            Image(systemName: "checkmark.circle")
                .font(.system(size: 40))
                .foregroundColor(.grokDim)
            Text("Issues")
                .font(.system(size: 16, weight: .semibold))
                .foregroundColor(.grokText)
                .padding(.top, 12)
            Text("Select a repo to view issues")
                .font(.system(size: 12))
                .foregroundColor(.grokMuted)
            Spacer()
        }
    }
}
