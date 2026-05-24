import SwiftUI
import AppKit

struct GlassmorphismBackground: NSViewRepresentable {
    var material: NSVisualEffectView.Material = .underWindowBackground
    var blending: NSVisualEffectView.BlendingMode = .behindWindow
    var cornerRadius: CGFloat = 20

    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = material
        view.blendingMode = blending
        view.state = .active
        view.wantsLayer = true
        view.layer?.cornerRadius = cornerRadius
        view.layer?.masksToBounds = true
        view.appearance = NSAppearance(named: .darkAqua)

        let tint = NSView()
        tint.wantsLayer = true
        tint.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.25).cgColor
        tint.layer?.cornerRadius = cornerRadius
        tint.layer?.masksToBounds = true
        tint.autoresizingMask = [.width, .height]
        view.addSubview(tint)

        return view
    }

    func updateNSView(_ nsView: NSVisualEffectView, context: Context) {
        nsView.material = material
        nsView.blendingMode = blending
        nsView.layer?.cornerRadius = cornerRadius
        if let tint = nsView.subviews.first {
            tint.layer?.cornerRadius = cornerRadius
        }
    }
}

struct GlassmorphismCard<Content: View>: View {
    let cornerRadius: CGFloat
    let content: Content

    init(cornerRadius: CGFloat = 16, @ViewBuilder content: () -> Content) {
        self.cornerRadius = cornerRadius
        self.content = content()
    }

    var body: some View {
        content
            .padding()
            .background(
                RoundedRectangle(cornerRadius: cornerRadius)
                    .fill(Color.grokSurface)
                    .background(
                        GlassmorphismBackground(material: .popover, blending: .withinWindow, cornerRadius: cornerRadius)
                    )
            )
            .overlay(
                RoundedRectangle(cornerRadius: cornerRadius)
                    .stroke(Color.grokBorder, lineWidth: 1)
            )
    }
}
