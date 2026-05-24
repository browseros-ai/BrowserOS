import SwiftUI

// MARK: - Grok.com Monochrome Palette

extension Color {
    static let grokBackground = Color.black
    static let grokSurface = Color(white: 0.04)
    static let grokElevated = Color(white: 0.07)
    static let grokBorder = Color(white: 0.12)
    static let grokDivider = Color(white: 0.15)
    static let grokTextPrimary = Color.white
    static let grokTextSecondary = Color(white: 0.55)
    static let grokTextMuted = Color(white: 0.35)
    static let grokAccent = Color.white
    static let grokAccentHover = Color(white: 0.8)

    // Legacy aliases for existing code compatibility
    static let triosGold = grokAccent
    static let triosBackground = grokBackground
    static let triosCardBackground = grokSurface
    static let triosReasoningBackground = grokElevated
    static let triosToolBackground = grokElevated
    static let triosSuccessBackground = grokElevated
    static let triosErrorBackground = Color(white: 0.04)
}

// MARK: - Corner Radius Style

extension View {
    func triosBubble(radius: CGFloat = 18, style: RoundedCornerStyle = .continuous) -> some View {
        clipShape(RoundedRectangle(cornerRadius: radius, style: style))
    }
}
