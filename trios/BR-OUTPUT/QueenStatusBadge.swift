import SwiftUI

struct QueenStatusBadge: View {
    @ObservedObject var viewModel: QueenStatusViewModel
    @State private var showSheet = false

    var body: some View {
        Button(action: {
            showSheet = true
        }) {
            HStack(spacing: 4) {
                Text("[Q]")
                    .font(.system(size: 13))
                Circle()
                    .fill(statusColor)
                    .frame(width: 6, height: 6)
            }
        }
        .buttonStyle(.plain)
        .sheet(isPresented: $showSheet) {
            QueenQuickActionsSheet(viewModel: viewModel, isPresented: $showSheet)
        }
    }

    private var statusColor: Color {
        switch viewModel.overallStatus {
        case .healthy: return Color.green
        case .warning: return Color.yellow
        case .down: return Color.red
        case .unknown: return Color.grokDim
        }
    }
}
