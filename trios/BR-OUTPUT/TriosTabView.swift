import SwiftUI

enum TriosTab: String, CaseIterable {
    case chat = "Chat"
    case github = "GitHub"
    case gitbutler = "GitButler"
    case terminal = "Terminal"
    case browseros = "BrowserOS"
    case queen = "Queen"

    var width: CGFloat {
        switch self {
        case .chat: return 400
        case .browseros: return 600
        case .queen: return 500
        default: return 700
        }
    }
}

struct TriosTabView: View {
    let viewModel: ChatViewModel
    let onWidthChange: (CGFloat) -> Void

    @State private var selectedTab: TriosTab = .chat

    var body: some View {
        VStack(spacing: 0) {
            tabSwitcher
            Divider().overlay(Color.grokBorder)
            content
        }
        .background(
            GlassmorphismBackground(material: .underWindowBackground, blending: .behindWindow, cornerRadius: 20)
        )
        .onChange(of: selectedTab) { newTab in
            onWidthChange(newTab.width)
        }
    }

    private var tabSwitcher: some View {
        HStack(spacing: 4) {
            ForEach(TriosTab.allCases, id: \.self) { tab in
                tabButton(for: tab)
            }
        }
        .padding(4)
        .background(
            GlassmorphismBackground(material: .popover, blending: .withinWindow, cornerRadius: 12)
                .overlay(
                    RoundedRectangle(cornerRadius: 12)
                        .stroke(Color.grokBorder.opacity(0.5), lineWidth: 1)
                )
        )
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private func tabButton(for tab: TriosTab) -> some View {
        let isSelected = selectedTab == tab
        return Button(action: {
            selectedTab = tab
        }) {
            Text(tab.rawValue)
                .font(.system(size: 11, weight: isSelected ? .semibold : .regular))
                .foregroundColor(isSelected ? .grokText : .grokMuted)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 5)
                .background(
                    isSelected
                        ? Color.grokElevated.opacity(0.6)
                        : Color.clear
                )
                .cornerRadius(6)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder
    private var content: some View {
        switch selectedTab {
        case .chat:
            ChatPanelView(viewModel: viewModel)
        case .github:
            GitHubDashboardView()
        case .gitbutler:
            GitButlerPanelView()
        case .terminal:
            TerminalTabView()
        case .browseros:
            BrowserOSBridgeView()
        case .queen:
            QueenTabView()
        }
    }
}