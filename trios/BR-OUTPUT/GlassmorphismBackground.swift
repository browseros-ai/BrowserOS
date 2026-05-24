import SwiftUI
import AppKit

struct GlassmorphismBackground: NSViewRepresentable {
    var cornerRadius: CGFloat = 20

    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.black.cgColor
        view.layer?.cornerRadius = cornerRadius
        view.layer?.masksToBounds = true
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        nsView.layer?.cornerRadius = cornerRadius
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
                    .fill(Color(NSColor.controlBackgroundColor))
            )
    }
}
