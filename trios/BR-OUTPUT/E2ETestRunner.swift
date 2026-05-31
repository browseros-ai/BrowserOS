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
    
    static func runHealthTest() {
        guard let url = URL(string: ProjectPaths.browserOSHealthURL) else {
            print("FAIL")
            return
        }
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
        print(result ? "PASS" : "FAIL")
    }
}

// Usage: call E2ETestRunner.runHealthTest() or E2ETestRunner.click(at:) from test targets