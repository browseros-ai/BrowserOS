import SwiftUI

struct ChatPanelView: View {
    @ObservedObject var viewModel: ChatViewModel

    private let suggestedPrompts = [
        "Analyze current page",
        "Search the web for...",
        "Create a GitHub task",
        "Summarize this tab",
    ]

    var body: some View {
        VStack(spacing: 0) {
            headerBar
            Divider().overlay(Color.gray.opacity(0.3))
            messageArea
            inputBar
        }
        .background(Color.black)
    }

    // MARK: - Header

    private var headerBar: some View {
        HStack(spacing: 12) {
            if let logoURL = Bundle.main.url(forResource: "logo", withExtension: "png"),
               let nsImage = NSImage(contentsOf: logoURL) {
                Image(nsImage: nsImage)
                    .resizable()
                    .renderingMode(.template)
                    .aspectRatio(contentMode: .fit)
                    .frame(width: 24, height: 20)
                    .foregroundColor(.primary)
            } else if let logoPath = "/Users/playra/BrowserOS-full/trios/logo.png" as String?,
                      FileManager.default.fileExists(atPath: logoPath),
                      let nsImage = NSImage(contentsOfFile: logoPath) {
                Image(nsImage: nsImage)
                    .resizable()
                    .renderingMode(.template)
                    .aspectRatio(contentMode: .fit)
                    .frame(width: 24, height: 20)
                    .foregroundColor(.primary)
            }

            Spacer()

            HStack(spacing: 10) {
                StatusDot(isOn: viewModel.isServerReachable, label: "Online", color: viewModel.isServerReachable ? .primary : .gray)
                StatusDot(isOn: viewModel.isA2ARegistered, label: "A2A", color: viewModel.isA2ARegistered ? .primary : .gray)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    // MARK: - Messages / Empty State

    private var messageArea: some View {
        ScrollViewReader { proxy in
            ScrollView {
                if viewModel.messages.isEmpty {
                    emptyStateView
                } else {
                    LazyVStack(spacing: 0) {
                        ForEach(viewModel.messages) { message in
                            MessageBubbleView(
                                message: message,
                                onTaskAction: { taskId, state in
                                    Task { await viewModel.updateTaskState(id: taskId, state: state) }
                                },
                                onRegenerate: {
                                    Task { await viewModel.regenerateLastResponse() }
                                },
                                onFeedback: { isPositive in
                                    Task { await viewModel.sendFeedback(messageId: message.id, isPositive: isPositive) }
                                }
                            )
                            .id(message.id)
                        }
                    }
                }
            }
            .onChange(of: viewModel.messages.count) {
                if let last = viewModel.messages.last {
                    withAnimation {
                        proxy.scrollTo(last.id, anchor: .bottom)
                    }
                }
            }
            .onChange(of: viewModel.messages.last?.content.count) {
                if let last = viewModel.messages.last {
                    withAnimation {
                        proxy.scrollTo(last.id, anchor: .bottom)
                    }
                }
            }
        }
    }

    private var emptyStateView: some View {
        VStack(spacing: 20) {
            Spacer()

            if let logoURL = Bundle.main.url(forResource: "logo", withExtension: "png"),
               let nsImage = NSImage(contentsOf: logoURL) {
                Image(nsImage: nsImage)
                    .resizable()
                    .renderingMode(.template)
                    .aspectRatio(contentMode: .fit)
                    .frame(width: 52, height: 44)
                    .foregroundColor(.primary)
                    .opacity(0.9)
            } else if let logoPath = "/Users/playra/BrowserOS-full/trios/logo.png" as String?,
                      FileManager.default.fileExists(atPath: logoPath),
                      let nsImage = NSImage(contentsOfFile: logoPath) {
                Image(nsImage: nsImage)
                    .resizable()
                    .renderingMode(.template)
                    .aspectRatio(contentMode: .fit)
                    .frame(width: 52, height: 44)
                    .foregroundColor(.primary)
                    .opacity(0.9)
            }

            Text("TRIOS")
                .font(.system(size: 36, weight: .bold, design: .default))
                .foregroundColor(.primary)

            Text("How can I help?")
                .font(.system(size: 16, weight: .regular, design: .default))
                .foregroundColor(.secondary)

            Spacer()
        }
        .padding(.vertical, 60)
    }

    // MARK: - Input Bar

    private var inputBar: some View {
        VStack(spacing: 0) {
            Divider().overlay(Color.gray.opacity(0.3))
            HStack(spacing: 12) {
                TextField("Ask anything...", text: $viewModel.inputText, axis: .vertical)
                    .textFieldStyle(PlainTextFieldStyle())
                    .font(.body)
                    .foregroundColor(.primary)
                    .lineLimit(1...5)
                    .onSubmit {
                        triggerSend()
                    }

                Button(action: {
                    triggerSend()
                }) {
                    Image(systemName: viewModel.state == .idle ? "arrow.up" : "stop.fill")
                        .font(.system(size: 16, weight: .semibold))
                        .foregroundColor(viewModel.inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? .gray : .primary)
                        .frame(width: 32, height: 32)
                        .background(
                            Circle()
                                .fill(viewModel.inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? Color.clear : Color(NSColor.controlBackgroundColor).opacity(0.8))
                        )
                }
                .buttonStyle(PlainButtonStyle())
                .disabled(viewModel.inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
        }
        .padding(.bottom, 20)
    }

    private func triggerSend() {
        Task {
            await viewModel.sendMessage()
        }
    }
}

// MARK: - Status Dot

private struct StatusDot: View {
    let isOn: Bool
    let label: String?
    let color: Color

    var body: some View {
        HStack(spacing: 4) {
            Circle()
                .fill(isOn ? color : Color.gray)
                .frame(width: 6, height: 6)
            if let label = label {
                Text(label)
                    .font(.caption2)
                    .foregroundColor(.secondary)
            }
        }
    }
}
