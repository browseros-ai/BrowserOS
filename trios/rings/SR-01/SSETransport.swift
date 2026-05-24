import Foundation

actor SSETransport: ChatTransportProtocol {
    private let serverURL: URL
    private var session: URLSession

    init(serverURL: URL = URL(string: "http://127.0.0.1:9105/chat")!) {
        let config = URLSessionConfiguration.default
        config.timeoutIntervalForRequest = 300
        config.timeoutIntervalForResource = 300
        self.serverURL = serverURL
        self.session = URLSession(configuration: config)
    }

    func sendMessage(body: Data) async throws -> AsyncStream<SSEEvent> {
        var request = URLRequest(url: serverURL)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        request.httpBody = body

        NSLog("[SSETransport] POST \(serverURL.absoluteString)")
        let (bytes, response) = try await session.bytes(for: request)

        guard let httpResponse = response as? HTTPURLResponse else {
            NSLog("[SSETransport] non-HTTP response")
            throw TransportError.invalidResponse
        }
        NSLog("[SSETransport] HTTP status: \(httpResponse.statusCode)")
        guard (200...299).contains(httpResponse.statusCode) else {
            throw TransportError.invalidResponse
        }

        return AsyncStream { continuation in
            let readTask = Task {
                do {
                    for try await line in bytes.lines {
                        if let event = SSEEventParser.parse(line: line) {
                            continuation.yield(event)
                            if case .finish = event {
                                continuation.finish()
                                return
                            }
                            if case .abort = event {
                                continuation.finish()
                                return
                            }
                            if case .error = event {
                                continuation.finish()
                                return
                            }
                        }
                    }
                    continuation.finish()
                } catch {
                    continuation.yield(.error(id: "", message: error.localizedDescription))
                    continuation.finish()
                }
            }
            continuation.onTermination = { _ in
                readTask.cancel()
            }
        }
    }

    func cancel() async {
        session.invalidateAndCancel()
        let config = URLSessionConfiguration.default
        config.timeoutIntervalForRequest = 300
        config.timeoutIntervalForResource = 300
        session = URLSession(configuration: config)
    }
}

enum TransportError: Error {
    case invalidResponse
    case connectionFailed
}
