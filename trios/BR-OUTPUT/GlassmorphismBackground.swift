import SwiftUI
import AppKit

struct GlassmorphismBackground: NSViewRepresentable {
    var material: NSVisualEffectView.Material = .fullScreenUI
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

        // Apple-style subtle dark tint over blur
        let tint = NSView()
        tint.wantsLayer = true
        tint.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.12).cgColor
        tint.layer?.cornerRadius = cornerRadius
        tint.layer?.masksToBounds = true
        tint.autoresizingMask = [.width, .height]
        view.addSubview(tint)

        // Frosted edge stroke
        let border = NSView()
        border.wantsLayer = true
        border.layer?.borderWidth = 0.5
        border.layer?.borderColor = NSColor.white.withAlphaComponent(0.15).cgColor
        border.layer?.cornerRadius = cornerRadius
        border.layer?.masksToBounds = true
        border.autoresizingMask = [.width, .height]
        view.addSubview(border)

        return view
    }

    func updateNSView(_ nsView: NSVisualEffectView, context: Context) {
        nsView.material = material
        nsView.blendingMode = blending
        nsView.layer?.cornerRadius = cornerRadius
        for subview in nsView.subviews {
            subview.layer?.cornerRadius = cornerRadius
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
