import SwiftUI

/// Mesh network status and control tab for the Trios sidebar.
struct MeshTabView: View {
    @StateObject private var viewModel = MeshStatusViewModel()
    @State private var selectedPeer: UInt32 = 2

    var body: some View {
        VStack(spacing: 0) {
            headerBar
            Divider().overlay(Color.grokBorder)
            ScrollView {
                LazyVStack(spacing: 12) {
                    statusCard
                    if !viewModel.neighbors.isEmpty {
                        neighborsSection
                    }
                    if !viewModel.routes.isEmpty {
                        routesSection
                    }
                    if !viewModel.sessions.isEmpty {
                        sessionsSection
                    }
                    metricsSection
                    controlsSection
                    if let error = viewModel.lastError {
                        errorBadge(error)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 12)
            }
        }
        .background(Color.clear)
        .onAppear {
            viewModel.startPolling()
        }
        .onDisappear {
            viewModel.stopPolling()
        }
    }

    // MARK: - Header

    private var headerBar: some View {
        HStack(spacing: 8) {
            Image(systemName: "antenna.radiowaves.left.and.right")
                .font(.system(size: 13, weight: .semibold))
                .foregroundColor(.grokText)
            Text("Mesh")
                .font(.system(size: 13, weight: .semibold))
                .foregroundColor(.grokText)
            Spacer()
            if viewModel.isLoading {
                ProgressView()
                    .scaleEffect(0.6)
            }
            Button(action: {
                Task { await viewModel.refresh() }
            }) {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 11))
                    .foregroundColor(.grokMuted)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    // MARK: - Status Card

    private var statusCard: some View {
        HStack(spacing: 12) {
            Circle()
                .fill(viewModel.isReachable ? Color.green : Color.red)
                .frame(width: 10, height: 10)
            VStack(alignment: .leading, spacing: 2) {
                Text("Node \(viewModel.nodeId)")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundColor(.grokText)
                Text(viewModel.isReachable ? "clade-meshd reachable" : "clade-meshd unreachable")
                    .font(.system(size: 10))
                    .foregroundColor(viewModel.isReachable ? Color.green : Color.red)
            }
            Spacer()
        }
        .padding(10)
        .background(Color.grokElevated.opacity(0.25))
        .cornerRadius(10)
        .overlay(
            RoundedRectangle(cornerRadius: 10)
                .stroke(Color.grokBorder.opacity(0.3), lineWidth: 1)
        )
    }

    // MARK: - Neighbors

    private var neighborsSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Neighbors")
                .font(.system(size: 11, weight: .semibold))
                .foregroundColor(.grokMuted)
            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 8) {
                ForEach(viewModel.neighbors) { neighbor in
                    neighborCard(neighbor)
                }
            }
        }
    }

    private func neighborCard(_ neighbor: MeshNeighbor) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Image(systemName: "network")
                    .font(.system(size: 12))
                    .foregroundColor(.grokMuted)
                Text("Node \(neighbor.id)")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundColor(.grokText)
                Spacer()
                Circle()
                    .fill(neighbor.statusColor.swiftUIColor)
                    .frame(width: 8, height: 8)
            }
            Text("ETX: \(String(format: "%.2f", neighbor.etx)) (\(neighbor.etx_label))")
                .font(.system(size: 10))
                .foregroundColor(.grokDim)
        }
        .padding(10)
        .background(Color.grokElevated.opacity(0.2))
        .cornerRadius(8)
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(Color.grokBorder.opacity(0.3), lineWidth: 1)
        )
    }

    // MARK: - Routes

    private var routesSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Learned Routes")
                .font(.system(size: 11, weight: .semibold))
                .foregroundColor(.grokMuted)
            ForEach(viewModel.routes) { route in
                HStack(spacing: 8) {
                    Text("-> \(route.destination)")
                        .font(.system(size: 11))
                        .foregroundColor(.grokText)
                    Spacer()
                    if let nextHop = route.next_hop, let etx = route.path_etx {
                        Text("via \(nextHop) / ETX \(String(format: "%.2f", etx))")
                            .font(.system(size: 10))
                            .foregroundColor(.grokDim)
                    } else {
                        Text("unreachable")
                            .font(.system(size: 10))
                            .foregroundColor(Color.red)
                    }
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(Color.grokElevated.opacity(0.2))
                .cornerRadius(6)
            }
        }
    }

    // MARK: - Sessions

    private var sessionsSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Crypto Sessions")
                .font(.system(size: 11, weight: .semibold))
                .foregroundColor(.grokMuted)
            FlowLayout(spacing: 8) {
                ForEach(viewModel.sessions) { session in
                    HStack(spacing: 4) {
                        Image(systemName: session.has_session ? "lock.fill" : "lock.open")
                            .font(.system(size: 10))
                            .foregroundColor(session.has_session ? Color.green : Color.red)
                        Text("Peer \(session.peer)")
                            .font(.system(size: 10))
                            .foregroundColor(.grokText)
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(Color.grokElevated.opacity(0.3))
                    .cornerRadius(6)
                }
            }
        }
    }

    // MARK: - Metrics

    private var metricsSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Convergence Metrics")
                .font(.system(size: 11, weight: .semibold))
                .foregroundColor(.grokMuted)
            HStack(spacing: 8) {
                metricChip(
                    label: "Link loss",
                    value: viewModel.metrics.link_loss_to_reroute_ms
                )
                metricChip(
                    label: "Node off",
                    value: viewModel.metrics.node_off_to_reroute_ms
                )
            }
        }
    }

    private func metricChip(label: String, value: Float?) -> some View {
        VStack(spacing: 2) {
            Text(value.map { String(format: "%.0f ms", $0) } ?? "-")
                .font(.system(size: 12, weight: .semibold))
                .foregroundColor(.grokText)
            Text(label)
                .font(.system(size: 9))
                .foregroundColor(.grokDim)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 6)
        .background(Color.grokElevated.opacity(0.3))
        .cornerRadius(6)
    }

    // MARK: - Controls

    private var controlsSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Simulate")
                .font(.system(size: 11, weight: .semibold))
                .foregroundColor(.grokMuted)
            HStack(spacing: 8) {
                peerStepper
                Spacer()
            }
            HStack(spacing: 8) {
                actionButton("Seed peer") {
                    Task { await viewModel.seedPeer(selectedPeer) }
                }
                actionButton("Hello") {
                    Task { await viewModel.hello(peer: selectedPeer) }
                }
                actionButton("Observe") {
                    Task { await viewModel.observe(peer: selectedPeer, weHeard: true, theyHeard: true) }
                }
            }
            HStack(spacing: 8) {
                actionButton("Force dead", isDestructive: true) {
                    Task {
                        await viewModel.linkLoss()
                        await viewModel.forceDead(selectedPeer)
                        await viewModel.reroute()
                    }
                }
                actionButton("Reset metrics") {
                    Task { await viewModel.reroute() }
                }
            }
        }
    }

    private var peerStepper: some View {
        HStack(spacing: 8) {
            Text("Target peer:")
                .font(.system(size: 11))
                .foregroundColor(.grokText)
            Button(action: { if selectedPeer > 1 { selectedPeer -= 1 } }) {
                Image(systemName: "minus")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundColor(.grokText)
            }
            .buttonStyle(.plain)
            Text("\(selectedPeer)")
                .font(.system(size: 12, weight: .semibold))
                .foregroundColor(.grokText)
                .frame(width: 32)
            Button(action: { selectedPeer += 1 }) {
                Image(systemName: "plus")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundColor(.grokText)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(Color.grokElevated.opacity(0.3))
        .cornerRadius(6)
    }

    private func actionButton(_ title: String, isDestructive: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(.system(size: 10, weight: .semibold))
                .foregroundColor(isDestructive ? Color.red : .grokAccent)
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(Color.grokElevated.opacity(0.6))
                .cornerRadius(6)
        }
        .buttonStyle(.plain)
    }

    private func errorBadge(_ message: String) -> some View {
        HStack(spacing: 6) {
            Image(systemName: "exclamationmark.triangle")
                .font(.system(size: 11))
                .foregroundColor(Color.red)
            Text(message)
                .font(.system(size: 10))
                .foregroundColor(Color.red)
                .lineLimit(2)
            Spacer()
        }
        .padding(8)
        .background(Color.red.opacity(0.1))
        .cornerRadius(6)
        .overlay(
            RoundedRectangle(cornerRadius: 6)
                .stroke(Color.red.opacity(0.3), lineWidth: 1)
        )
    }
}

// MARK: - Flow Layout

/// A simple horizontal wrap layout for session chips.
struct FlowLayout: Layout {
    var spacing: CGFloat = 8

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let result = FlowResult(in: proposal.width ?? 0, subviews: subviews, spacing: spacing)
        return result.size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let result = FlowResult(in: bounds.width, subviews: subviews, spacing: spacing)
        for (index, subview) in subviews.enumerated() {
            subview.place(at: CGPoint(x: bounds.minX + result.frames[index].minX,
                                      y: bounds.minY + result.frames[index].minY),
                          proposal: .unspecified)
        }
    }

    struct FlowResult {
        var size: CGSize = .zero
        var frames: [CGRect] = []

        init(in maxWidth: CGFloat, subviews: Subviews, spacing: CGFloat) {
            var x: CGFloat = 0
            var y: CGFloat = 0
            var lineHeight: CGFloat = 0
            for subview in subviews {
                let size = subview.sizeThatFits(.unspecified)
                if x + size.width > maxWidth, x > 0 {
                    x = 0
                    y += lineHeight + spacing
                    lineHeight = 0
                }
                frames.append(CGRect(x: x, y: y, width: size.width, height: size.height))
                x += size.width + spacing
                lineHeight = max(lineHeight, size.height)
            }
            self.size = CGSize(width: maxWidth, height: y + lineHeight)
        }
    }
}
