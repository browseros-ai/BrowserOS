import SwiftUI

struct BrowserOSBridgeView: View {
    @StateObject private var viewModel = BrowserOSChatViewModel()
    @State private var inputText: String = ""
    
    var body: some View {
        VStack(spacing: 0) {
            QueenHeaderView(statusText: viewModel.queenStatusText)
                .padding(.horizontal).padding(.top, 8)
            
            ConnectionStatusBar(isConnected: viewModel.isBrowserOSConnected)
            
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 12) {
                        ForEach(viewModel.messages) { message in
                            MessageBubble(message: message)
                                .id(message.id)
                        }
                        if viewModel.isStreaming {
                            TypingIndicatorView().id("typing")
                        }
                    }
                    .padding(.horizontal).padding(.vertical, 8)
                }
                .onChange(of: viewModel.messages.count) { _ in
                    withAnimation { proxy.scrollTo(viewModel.messages.last?.id, anchor: .bottom) }
                }
            }
            
            HStack(spacing: 8) {
                TextField("Command BrowserOS...", text: $inputText)
                    .textFieldStyle(PlainTextFieldStyle())
                    .padding(10)
                    .background(Color(.systemGray).opacity(0.3))
                    .cornerRadius(20)
                
                Button(action: {
                    guard !inputText.isEmpty else { return }
                    viewModel.sendMessage(inputText)
                    inputText = ""
                }) {
                    Image(systemName: "paperplane.fill")
                        .foregroundColor(.accentColor)
                        .padding(10)
                        .background(Color.accentColor.opacity(0.1))
                        .clipShape(Circle())
                }
                .buttonStyle(PlainButtonStyle())
            }
            .padding(.horizontal).padding(.bottom, 8)
        }
        .background(Color(.windowBackgroundColor).opacity(0.9))
    }
}

struct QueenHeaderView: View {
    let statusText: String
    var body: some View {
        HStack {
            Image(systemName: "crown.fill").foregroundColor(.yellow)
            Text("BrowserOS Agent").font(.headline)
            Spacer()
            Text(statusText).font(.caption).foregroundColor(.secondary)
        }
        .padding(.vertical, 4)
    }
}

struct ConnectionStatusBar: View {
    let isConnected: Bool
    var body: some View {
        HStack {
            Circle().fill(isConnected ? Color.green : Color.red).frame(width: 8, height: 8)
            Text(isConnected ? "MCP Connected" : "MCP Disconnected")
                .font(.caption2).foregroundColor(.secondary)
            Spacer()
        }
        .padding(.horizontal).padding(.vertical, 2)
    }
}

struct MessageBubble: View {
    let message: BrowserOSChatMessage
    var body: some View {
        HStack {
            if message.role == .user { Spacer() }
            VStack(alignment: .leading, spacing: 4) {
                Text(message.content)
                    .font(.body)
                    .padding(12)
                    .background(message.role == .user ? Color.accentColor.opacity(0.8) : Color(.systemGray).opacity(0.3))
                    .foregroundColor(message.role == .user ? .white : .primary)
                    .cornerRadius(16)
                if !message.toolCalls.isEmpty {
                    ForEach(message.toolCalls, id: \.name) { tool in
                        ToolCallMiniCard(tool: tool)
                    }
                }
            }
            if message.role == .assistant || message.role == .system { Spacer() }
        }
    }
}

struct ToolCallMiniCard: View {
    let tool: BrowserOSToolCall
    @State private var isExpanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Image(systemName: "hammer.fill")
                    .foregroundColor(.purple)
                    .font(.caption)
                Text(tool.name)
                    .font(.caption)
                    .fontWeight(.medium)
                Spacer()
                Button(action: { isExpanded.toggle() }) {
                    Image(systemName: isExpanded ? "chevron.up" : "chevron.down")
                        .font(.caption2)
                }
                .buttonStyle(PlainButtonStyle())
            }
            if isExpanded, let result = tool.result {
                Text(result)
                    .font(.caption2)
                    .foregroundColor(.secondary)
                    .padding(4)
                    .background(Color(.systemGray).opacity(0.3))
                    .cornerRadius(6)
            }
        }
        .padding(8)
        .background(Color.purple.opacity(0.1))
        .cornerRadius(8)
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(Color.purple.opacity(0.3), lineWidth: 1)
        )
    }
}

struct ToolCallsPanel: View {
    let toolCalls: [BrowserOSChatViewModel.ToolCallRecord]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Active Tools")
                .font(.caption)
                .fontWeight(.semibold)
                .foregroundColor(.secondary)
                .padding(.horizontal, 8)
                .padding(.top, 4)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(toolCalls) { tool in
                        ToolCallStatusCard(tool: tool)
                    }
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
            }
        }
        .background(Color(.systemGray).opacity(0.2))
        .cornerRadius(8)
    }
}

struct ToolCallStatusCard: View {
    let tool: BrowserOSChatViewModel.ToolCallRecord

    var body: some View {
        HStack(spacing: 4) {
            Circle()
                .fill(statusColor)
                .frame(width: 6, height: 6)
            Text(tool.name)
                .font(.caption2)
            if tool.status == .running {
                ProgressView()
                    .scaleEffect(0.6)
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(Color(.systemGray).opacity(0.3))
        .cornerRadius(12)
    }

    private var statusColor: Color {
        switch tool.status {
        case .running: return .yellow
        case .completed: return .green
        case .failed: return .red
        }
    }
}