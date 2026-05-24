import SwiftUI

struct TypingIndicatorView: View {
    @State private var phase: CGFloat = 0

    var body: some View {
        HStack(spacing: 4) {
            ForEach(0..<3, id: \.self) { index in
                Circle()
                    .fill(Color.gray.opacity(0.6))
                    .frame(width: 6, height: 6)
                    .scaleEffect(scale(for: index))
                    .animation(
                        Animation.easeInOut(duration: 0.6)
                            .repeatForever(autoreverses: true)
                            .delay(Double(index) * 0.2),
                        value: phase
                    )
            }
        }
        .padding(.horizontal, 4)
        .onAppear {
            phase = 1
        }
    }

    private func scale(for index: Int) -> CGFloat {
        let base = CGFloat(0.6)
        let amplitude = CGFloat(0.4)
        let offset = Double(index) * 0.2
        let value = sin((Double(phase) * .pi * 2) + offset)
        return base + amplitude * CGFloat(abs(value))
    }
}
