import SwiftUI

enum MainTab: String, CaseIterable {
    case chat = "Chat"
    case git = "Git"
    case terminal = "Terminal"
    case mesh = "Mesh"
    case queen = "Queen"
    case settings = "Settings"

    var icon: String {
        switch self {
        case .chat: return "bubble.left.fill"
        case .git: return "arrow.triangle.branch"
        case .terminal: return "terminal.fill"
        case .mesh: return "antenna.radiowaves.left.and.right"
        case .queen: return "crown.fill"
        case .settings: return "gear"
        }
    }
}

struct TriosTabView: View {
    @ObservedObject var viewModel: ChatViewModel
    @State private var selectedTab: MainTab = .chat

    var body: some View {
        VStack(spacing: 0) {
            titleBar
            tabBar
            Divider().overlay(Color.grokBorder)
            content
        }
        .background(Color.clear)
        .sheet(isPresented: $viewModel.showHistory) {
            historySheet
        }
    }

    // MARK: - Title Bar

    private var titleBar: some View {
        HStack(spacing: 12) {
            logoView(size: CGSize(width: 22, height: 18))

            Text("TRIOS AGENT")
                .font(.system(size: 12, weight: .bold, design: .default))
                .foregroundColor(.grokText)

            Spacer()

            HStack(spacing: 8) {
                Circle()
                    .fill(viewModel.isServerReachable ? Color.green : Color.grokDim)
                    .frame(width: 6, height: 6)
                Text(viewModel.isServerReachable ? "Online" : "Offline")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundColor(.grokMuted)

                if viewModel.isA2ARegistered {
                    Circle()
                        .fill(Color.blue)
                        .frame(width: 6, height: 6)
                    Text("A2A")
                        .font(.system(size: 10, weight: .medium))
                        .foregroundColor(.grokMuted)
                }
            }

            Button(action: {
                viewModel.newConversation()
            }) {
                Image(systemName: "plus")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundColor(.grokMuted)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
    }

    // MARK: - Tab Bar (Icons)

    private var tabBar: some View {
        HStack(spacing: 0) {
            ForEach(MainTab.allCases, id: \.self) { tab in
                tabButton(for: tab)
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
    }

    private func tabButton(for tab: MainTab) -> some View {
        let isSelected = selectedTab == tab
        return Button(action: { selectedTab = tab }) {
            HStack(spacing: 4) {
                Image(systemName: tab.icon)
                    .font(.system(size: 13, weight: isSelected ? .semibold : .regular))
                    .foregroundColor(isSelected ? .grokText : .grokMuted)
                if isSelected {
                    Text(tab.rawValue)
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundColor(.grokText)
                }
            }
            .padding(.horizontal, isSelected ? 10 : 8)
            .padding(.vertical, 6)
            .background(
                isSelected
                    ? Color.white.opacity(0.12)
                    : Color.clear
            )
            .cornerRadius(8)
        }
        .buttonStyle(.plain)
    }

    // MARK: - Content

    @ViewBuilder
    private var content: some View {
        switch selectedTab {
        case .chat:
            ChatPanelView(viewModel: viewModel)
        case .git:
            GitWorkspaceView()
        case .terminal:
            TerminalTabView()
        case .mesh:
            MeshTabView()
        case .queen:
            QueenTabView()
        case .settings:
            SettingsTabView()
        }
    }

    // MARK: - History Sheet

    private var historySheet: some View {
        VStack(spacing: 0) {
            HStack {
                Text("History")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundColor(.grokText)
                Spacer()
                Button(action: { viewModel.showHistory = false }) {
                    Image(systemName: "xmark")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundColor(.grokMuted)
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)

            Divider().overlay(Color.grokBorder)

            if viewModel.conversations.isEmpty {
                Text("No history yet")
                    .font(.system(size: 12))
                    .foregroundColor(.grokDim)
                    .padding(.top, 20)
            } else {
                List(viewModel.conversations) { conv in
                    Button(action: {
                        Task { await viewModel.switchConversation(id: conv.id) }
                    }) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(conv.title)
                                .font(.system(size: 12))
                                .foregroundColor(.grokText)
                                .lineLimit(1)
                            Text(conv.updatedAt, style: .relative)
                                .font(.system(size: 9))
                                .foregroundColor(.grokDim)
                        }
                    }
                    .buttonStyle(.plain)
                }
                .listStyle(.plain)
                .scrollContentBackground(.hidden)
            }

            Spacer()
        }
        .frame(width: 320, height: 400)
        .background(
            GlassmorphismBackground(material: .popover, blending: .withinWindow, cornerRadius: 16)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 16)
                .stroke(Color.grokBorder.opacity(0.4), lineWidth: 1)
        )
    }

    private func logoView(size: CGSize) -> some View {
        Group {
            if let pngURL = Bundle.main.url(forResource: "logo", withExtension: "png"),
               let nsImage = NSImage(contentsOf: pngURL) {
                Image(nsImage: nsImage)
                    .resizable()
                    .renderingMode(.template)
                    .aspectRatio(contentMode: .fit)
                    .frame(width: size.width, height: size.height)
                    .foregroundColor(.grokText)
            } else if FileManager.default.fileExists(atPath: ProjectPaths.logoPNG),
                      let nsImage = NSImage(contentsOfFile: ProjectPaths.logoPNG) {
                Image(nsImage: nsImage)
                    .resizable()
                    .renderingMode(.template)
                    .aspectRatio(contentMode: .fit)
                    .frame(width: size.width, height: size.height)
                    .foregroundColor(.grokText)
            }
        }
    }
}

// MARK: - Settings Placeholder

struct SettingsTabView: View {
    var body: some View {
        VStack(spacing: 16) {
            Spacer()
            Image(systemName: "gear")
                .font(.system(size: 40))
                .foregroundColor(.grokDim)
            Text("Settings")
                .font(.system(size: 16, weight: .semibold))
                .foregroundColor(.grokText)
            Text("Coming soon")
                .font(.system(size: 12))
                .foregroundColor(.grokMuted)
            Spacer()
        }
    }
}
