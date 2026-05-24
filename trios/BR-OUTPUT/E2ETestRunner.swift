import Foundation
import CoreGraphics

class E2ETestRunner {
    static func click(at point: CGPoint) {
        let down = CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown,
                          mouseCursorPosition: point, mouseButton: .left)
        down?.post(tap: .cghidEventTap)
        let up = CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp,
                        mouseCursorPosition: point, mouseButton: .left)
        up?.post(tap: .cghidEventTap)
    }
    
    static func pressKey(keyCode: CGKeyCode, modifiers: CGEventFlags = []) {
        let keyDown = CGEvent(keyboardEventSource: nil, virtualKey: keyCode, keyDown: true)
        keyDown?.flags = modifiers
        keyDown?.post(tap: .cghidEventTap)
        let keyUp = CGEvent(keyboardEventSource: nil, virtualKey: keyCode, keyDown: false)
        keyUp?.flags = modifiers
        keyUp?.post(tap: .cghidEventTap)
    }
    
    static func runHealthTest() -> Bool {
        let url = URL(string: "http://127.0.0.1:9105/health")!
        let semaphore = DispatchSemaphore(value: 0)
        var result = false
        let task = URLSession.shared.dataTask(with: url) { data, _, _ in
            if let data = data,
               let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let status = json["status"] as? String {
                result = status == "ok"
            }
            semaphore.signal()
        }
        task.resume()
        semaphore.wait()
        return result
    }
}