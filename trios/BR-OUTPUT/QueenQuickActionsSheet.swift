import SwiftUI

struct QueenQuickActionsSheet: View {
    @ObservedObject var viewModel: QueenStatusViewModel
    @Binding var isPresented: Bool

    var body: some View {
        VStack(spacing: 0) {
            headerBar
            Divider().overlay(Color.grokBorder)
            actionButtons
            Divider().overlay(Color.grokBorder)
            scrollableCards
            closeButton
        }
        .frame(width: 360, height: 350)
        .background(
            GlassmorphismBackground(material: .popover, blending: .withinWindow, cornerRadius: 16)
                .overlay(
                    RoundedRectangle(cornerRadius: 16)
                        .stroke(Color.grokBorder.opacity(0.4), lineWidth: 1)
                )
        )
    }

    private var headerBar: some View {
        HStack {
            Text("Queen Trinity")
                .font(.system(size: 14, weight: .semibold))
                .foregroundColor(.grokText)
            Spacer()
            if viewModel.isRunningAction {
                ProgressView()
                    .scaleEffect(0.6)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }

    private var actionButtons: some View {
        HStack(spacing: 8) {
            quickButton(icon: "play.fill", label: "Start", action: { viewModel.startTrios() })
            quickButton(icon: "arrow.clockwise", label: "MCP", action: { viewModel.restartMCP() })
            quickButton(icon: "clock.arrow.circlepath", label: "Cron", action: { viewModel.runCron() })
            quickButton(icon: "plus.bubble", label: "Chat", action: { isPresented = false })
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }

    private func quickButton(icon: String, label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            VStack(spacing: 4) {
                Image(systemName: icon)
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundColor(.grokText)
                Text(label)
                    .font(.system(size: 9, weight: .medium))
                    .foregroundColor(.grokMuted)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .background(Color.grokElevated.opacity(0.4))
            .cornerRadius(8)
        }
        .buttonStyle(.plain)
    }

    private var scrollableCards: some View {
        ScrollView {
            LazyVStack(spacing: 6) {
                ForEach(viewModel.components) { comp in
                    miniCard(comp)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
        }
    }

    private func miniCard(_ comp: StatusComponent) -> some View {
        HStack(spacing: 8) {
            Image(systemName: comp.icon)
                .font(.system(size: 12))
                .foregroundColor(.grokMuted)
                .frame(width: 20)

            Text(comp.name)
                .font(.system(size: 11, weight: .medium))
                .foregroundColor(.grokText)

            Spacer()

            Circle()
                .fill(statusColor(comp.status))
                .frame(width: 6, height: 6)

            Text(comp.detail)
                .font(.system(size: 10))
                .foregroundColor(.grokMuted)
                .lineLimit(1)

            if let action = comp.actionLabel {
                Button(action: {
                    runAction(for: comp.name)
                }) {
                    Text(action)
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundColor(.grokAccent)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Color.grokElevated.opacity(0.6))
                        .cornerRadius(4)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(Color.grokElevated.opacity(0.2))
        .cornerRadius(8)
    }

    private func statusColor(_ status: ComponentStatus) -> Color {
        switch status {
        case .healthy: return Color.green
        case .warning: return Color.yellow
        case .down: return Color.red
        case .unknown: return Color.grokDim
        }
    }

    private func runAction(for name: String) {
        switch name {
        case "TRIOS": viewModel.startTrios()
        case "MCP": viewModel.restartMCP()
        case "Agent": viewModel.restartAgentServer()
        case "Cron": viewModel.runCron()
        default: break
        }
    }

    private var closeButton: some View {
        Button(action: { isPresented = false }) {
            Text("Close")
                .font(.system(size: 12, weight: .semibold))
                .foregroundColor(.grokMuted)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 10)
                .background(Color.grokElevated.opacity(0.3))
                .cornerRadius(8)
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }
}
