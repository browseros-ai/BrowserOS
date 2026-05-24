import Foundation

// MARK: - SSEEvent Parser
/// Parses Server-Sent Events from BrowserOS MCP streaming endpoint
struct SSEEventParser {
    
    static func parse(data: Data) -> [SSEEvent] {
        guard let text = String(data: data, encoding: .utf8) else { return [] }
        
        var events: [SSEEvent] = []
        let lines = text.split(separator: "\n")
        
        var currentEvent: String?
        
        for line in lines {
            if line.hasPrefix("event: ") {
                let eventType = String(line.dropFirst(7))
                currentEvent = eventType
            } else if line.hasPrefix("data: ") {
                let payload = String(line.dropFirst(6))
                if let eventType = currentEvent {
                    events.append(SSEEvent(type: eventType, data: payload))
                }
            } else if line.isEmpty {
                currentEvent = nil
            }
        }
        
        return events
    }
}

struct SSEEvent {
    let type: String
    let data: String
    
    var isDone: Bool {
        data == "[DONE]"
    }
    
    var isToolCall: Bool {
        type == "tool_call" || data.contains(""type":"tool_call"")
    }
    
    var isDelta: Bool {
        type == "delta" || type == "inputTextDelta"
    }
}