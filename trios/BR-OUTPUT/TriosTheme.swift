import SwiftUI

// MARK: - Grok.com Monochrome Palette

extension Color {
    static let grokBackground = Color.black
    static let grokSurface  = Color(white: 0.06)
    static let grokElevated = Color(white: 0.10)
    static let grokBorder   = Color(white: 0.18)
    static let grokDivider  = Color(white: 0.22)
    static let grokText     = Color.white
    static let grokMuted    = Color(white: 0.55)
    static let grokDim      = Color(white: 0.40)
    static let grokAccent   = Color.white

    // Legacy aliases
    static let triosGold = grokAccent
    static let triosBackground = grokBackground
    static let triosCardBackground = grokSurface
    static let triosReasoningBackground = grokElevated
    static let triosToolBackground = grokElevated
    static let triosSuccessBackground = grokElevated
    static let triosErrorBackground = Color(white: 0.06)
}

// MARK: - Corner Radius Style

extension View {
    func triosBubble(radius: CGFloat = 18, style: RoundedCornerStyle = .continuous) -> some View {
        clipShape(RoundedRectangle(cornerRadius: radius, style: style))
    }
}
