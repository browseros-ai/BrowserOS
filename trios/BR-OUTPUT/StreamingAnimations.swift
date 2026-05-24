import SwiftUI

// MARK: - StreamingMessageBubble
/// Shows text appearing character-by-character with typing animation
struct StreamingMessageBubble: View {
    let fullText: String
    @State private var visibleCount: Int = 0
    let characterDelay: Double = 0.016 // 16ms per character = ~60 FPS
    
    var body: some View {
        Text(String(fullText.prefix(visibleCount)))
            .font(.body)
            .padding(12)
            .background(Color(.systemGray).opacity(0.3))
            .foregroundColor(.primary)
            .cornerRadius(16)
            .onAppear {
                startAnimation()
            }
    }
    
    private func startAnimation() {
        visibleCount = 0
        Timer.scheduledTimer(withTimeInterval: characterDelay, repeats: true) { timer in
            if visibleCount < fullText.count {
                visibleCount += 1
            } else {
                timer.invalidate()
            }
        }
    }
}

// MARK: - AnimatedTypingIndicator
/// Three dots with staggered bounce animation
struct AnimatedTypingIndicator: View {
    @State private var phase: Double = 0
    
    var body: some View {
        HStack(spacing: 4) {
            ForEach(0..<3) { i in
                Circle()
                    .fill(Color.secondary)
                    .frame(width: 6, height: 6)
                    .scaleEffect(scaleForDot(index: i))
                    .offset(y: offsetForDot(index: i))
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .onAppear {
            withAnimation(.linear(duration: 0.6).repeatForever(autoreverses: false)) {
                phase = 1.0
            }
        }
    }
    
    private func scaleForDot(index: Int) -> CGFloat {
        let staggeredPhase = (phase + Double(index) * 0.33).truncatingRemainder(dividingBy: 1.0)
        return 0.5 + 0.5 * CGFloat(sin(staggeredPhase * .pi * 2))
    }
    
    private func offsetForDot(index: Int) -> CGFloat {
        let staggeredPhase = (phase + Double(index) * 0.33).truncatingRemainder(dividingBy: 1.0)
        return -4 * CGFloat(sin(staggeredPhase * .pi * 2))
    }
}