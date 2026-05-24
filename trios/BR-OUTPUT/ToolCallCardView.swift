import SwiftUI

struct ToolCallCardView: View {
    let toolCall: ToolCall
    @State private var isExpanded: Bool = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button(action: { isExpanded.toggle() }) {
                HStack {
                    Image(systemName: toolCall.isComplete ? "checkmark.circle.fill" : "hammer.fill")
                        .foregroundColor(toolCall.isComplete ? .secondary : .gray)
                    Text(toolCall.name)
                        .font(.caption)
                        .fontWeight(.semibold)
                        .foregroundColor(.primary)
                    Spacer()
                    Image(systemName: isExpanded ? "chevron.up" : "chevron.down")
                        .font(.caption2)
                        .foregroundColor(.gray)
                }
            }
            .buttonStyle(PlainButtonStyle())

            if isExpanded {
                VStack(alignment: .leading, spacing: 4) {
                    if !toolCall.arguments.isEmpty {
                        Text("Arguments")
                            .font(.caption2)
                            .foregroundColor(.secondary)
                        Text(toolCall.arguments)
                            .font(.caption.monospaced())
                            .foregroundColor(.primary)
                            .padding(6)
                            .background(Color(NSColor.controlBackgroundColor).opacity(0.8))
                            .cornerRadius(6)
                    }
                    if let output = toolCall.output {
                        Text("Output")
                            .font(.caption2)
                            .foregroundColor(.secondary)
                        Text(output)
                            .font(.caption.monospaced())
                            .foregroundColor(.primary)
                            .padding(6)
                            .background(Color(NSColor.controlBackgroundColor).opacity(0.8))
                            .cornerRadius(6)
                    }
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(Color(NSColor.controlBackgroundColor).opacity(0.8))
        .cornerRadius(8)
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(Color.gray.opacity(0.3), lineWidth: 1)
        )
    }
}
