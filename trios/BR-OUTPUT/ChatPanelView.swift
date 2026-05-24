import SwiftUI

struct ChatPanelView: View {
    @ObservedObject var viewModel: ChatViewModel

    var body: some View {
        VStack(spacing: 0) {
            headerBar
            Divider().overlay(Color.grokDivider)
            messageArea
            inputBar
        }
        .background(Color.clear)
    }

    // MARK: - Header

    private var headerBar: some View {
        HStack(spacing: 12) {
            logoView(size: CGSize(width: 24, height: 20))

            Spacer()

            HStack(spacing: 10) {
                StatusDot(isOn: viewModel.isServerReachable, label: "Online", color: viewModel.isServerReachable ? .grokText : .grokDim)
                StatusDot(isOn: viewModel.isA2ARegistered, label: "A2A", color: viewModel.isA2ARegistered ? .grokText : .grokDim)
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

            logoView(size: CGSize(width: 52, height: 44))

            Text("TRIOS")
                .font(.system(size: 36, weight: .bold, design: .default))
                .foregroundColor(.grokText)

            Text("How can I help?")
                .font(.system(size: 16, weight: .regular, design: .default))
                .foregroundColor(.grokMuted)

            Spacer()
        }
        .padding(.vertical, 60)
    }

    // MARK: - Input Bar

    private var inputBar: some View {
        VStack(spacing: 0) {
            Divider().overlay(Color.grokDivider)
            HStack(spacing: 12) {
                TextField("Ask anything...", text: $viewModel.inputText, axis: .vertical)
                    .textFieldStyle(PlainTextFieldStyle())
                    .font(.system(size: 15, weight: .regular, design: .default))
                    .foregroundColor(.grokText)
                    .lineLimit(1...5)
                    .onSubmit {
                        triggerSend()
                    }

                Button(action: {
                    triggerSend()
                }) {
                    Image(systemName: viewModel.state == .idle ? "arrow.up" : "stop.fill")
                        .font(.system(size: 16, weight: .semibold))
                        .foregroundColor(viewModel.inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? .grokDim : .grokText)
                        .frame(width: 32, height: 32)
                        .background(
                            Circle()
                                .fill(viewModel.inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? Color.clear : Color.grokElevated)
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

// MARK: - Logo Helper

private func logoView(size: CGSize) -> some View {
    Group {
        if let svgURL = Bundle.main.url(forResource: "logo", withExtension: "svg"),
           let nsImage = NSImage(contentsOf: svgURL) {
            Image(nsImage: nsImage)
                .resizable()
                .renderingMode(.template)
                .aspectRatio(contentMode: .fit)
                .frame(width: size.width, height: size.height)
                .foregroundColor(.grokText)
        } else if let pngURL = Bundle.main.url(forResource: "logo", withExtension: "png"),
                  let nsImage = NSImage(contentsOf: pngURL) {
            Image(nsImage: nsImage)
                .resizable()
                .renderingMode(.template)
                .aspectRatio(contentMode: .fit)
                .frame(width: size.width, height: size.height)
                .foregroundColor(.grokText)
        } else if FileManager.default.fileExists(atPath: "/Users/playra/BrowserOS-full/trios/logo.svg"),
                  let nsImage = NSImage(contentsOfFile: "/Users/playra/BrowserOS-full/trios/logo.svg") {
            Image(nsImage: nsImage)
                .resizable()
                .renderingMode(.template)
                .aspectRatio(contentMode: .fit)
                .frame(width: size.width, height: size.height)
                .foregroundColor(.grokText)
        } else if FileManager.default.fileExists(atPath: "/Users/playra/BrowserOS-full/trios/logo.png"),
                  let nsImage = NSImage(contentsOfFile: "/Users/playra/BrowserOS-full/trios/logo.png") {
            Image(nsImage: nsImage)
                .resizable()
                .renderingMode(.template)
                .aspectRatio(contentMode: .fit)
                .frame(width: size.width, height: size.height)
                .foregroundColor(.grokText)
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
                .fill(isOn ? color : Color.grokDim)
                .frame(width: 6, height: 6)
            if let label = label {
                Text(label)
                    .font(.system(size: 11, weight: .medium, design: .default))
                    .foregroundColor(.grokMuted)
            }
        }
    }
}
