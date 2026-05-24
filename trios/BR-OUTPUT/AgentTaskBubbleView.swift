import SwiftUI

struct AgentTaskBubbleView: View {
    let task: AgentTask
    var onAccept: (() -> Void)?
    var onReject: (() -> Void)?
    var onComplete: (() -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            // Title row
            HStack(spacing: 8) {
                Image(systemName: iconName)
                    .foregroundColor(.secondary)
                    .font(.title3)

                VStack(alignment: .leading, spacing: 2) {
                    Text(task.title)
                        .font(.subheadline)
                        .fontWeight(.semibold)
                        .foregroundColor(.primary)

                    Text(task.description)
                        .font(.caption)
                        .foregroundColor(.secondary)
                        .lineLimit(3)
                }

                Spacer()

                VStack(alignment: .trailing, spacing: 4) {
                    stateBadge
                    priorityBadge
                }
            }

            // Action buttons
            if showActions {
                HStack(spacing: 8) {
                    if task.state == .pending {
                        Button("Accept") {
                            onAccept?()
                        }
                        .buttonStyle(TaskActionButtonStyle())

                        Button("Reject") {
                            onReject?()
                        }
                        .buttonStyle(TaskActionButtonStyle())
                    }

                    if task.state == .assigned || task.state == .inProgress {
                        Button("Complete") {
                            onComplete?()
                        }
                        .buttonStyle(TaskActionButtonStyle())
                    }
                }
            }
        }
        .padding(12)
        .background(Color(NSColor.controlBackgroundColor).opacity(0.6))
        .cornerRadius(8)
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(Color.gray.opacity(0.2), lineWidth: 1)
        )
    }

    private var showActions: Bool {
        task.state == .pending || task.state == .assigned || task.state == .inProgress
    }

    private var iconName: String {
        switch task.state {
        case .pending: return "clock.badge.questionmark"
        case .assigned: return "person.badge.clock"
        case .inProgress: return "hammer.circle.fill"
        case .completed: return "checkmark.circle.fill"
        case .failed: return "xmark.circle.fill"
        case .cancelled: return "minus.circle.fill"
        }
    }

    private var stateBadge: some View {
        Text(task.state.rawValue.capitalized)
            .font(.caption2)
            .fontWeight(.semibold)
            .foregroundColor(.secondary)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(Color(NSColor.controlBackgroundColor).opacity(0.8))
            .clipShape(Capsule())
    }

    private var priorityBadge: some View {
        Text(task.priority == .critical ? "CRIT" : String(task.priority.rawValue))
            .font(.caption2)
            .fontWeight(.bold)
            .foregroundColor(.secondary)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(Color(NSColor.controlBackgroundColor).opacity(0.8))
            .clipShape(Capsule())
    }
}

struct TaskActionButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.caption)
            .fontWeight(.semibold)
            .foregroundColor(.primary)
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(Color(NSColor.controlBackgroundColor).opacity(configuration.isPressed ? 0.4 : 0.8))
            .cornerRadius(8)
            .overlay(
                RoundedRectangle(cornerRadius: 8)
                    .stroke(Color.gray.opacity(0.2), lineWidth: 1)
            )
    }
}
