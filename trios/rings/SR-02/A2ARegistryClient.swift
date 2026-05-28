import Foundation

enum A2AError: Error, Equatable {
    case notRegistered
    case networkError(Error)
    case invalidResponse(Int)
    case decodingError

    static func == (lhs: A2AError, rhs: A2AError) -> Bool {
        switch (lhs, rhs) {
        case (.notRegistered, .notRegistered): return true
        case (.decodingError, .decodingError): return true
        case (.invalidResponse(let a), .invalidResponse(let b)): return a == b
        case (.networkError, .networkError): return false
        default: return false
        }
    }
}

actor A2ARegistryClient {
    private let serverURL: URL
    private let agentCard: AgentCard
    private var registered = false
    private var heartbeatTask: Task<Void, Never>?
    private let session: URLSession
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()

    init(serverURL: URL, agentCard: AgentCard, session: URLSession = .shared) {
        self.serverURL = serverURL
        self.agentCard = agentCard
        self.session = session
        encoder.keyEncodingStrategy = .convertToSnakeCase
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        decoder.dateDecodingStrategy = .iso8601
    }

    // MARK: - Registration

    func register() async throws {
        let url = serverURL.appendingPathComponent("a2a/register")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try encoder.encode(agentCard)

        let (_, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode) else {
            throw A2AError.invalidResponse((response as? HTTPURLResponse)?.statusCode ?? 0)
        }
        registered = true
    }

    func unregister() async throws {
        let url = serverURL.appendingPathComponent("a2a/unregister")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try encoder.encode(["agentId": agentCard.id.rawValue] as [String: String])

        let (_, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode) else {
            throw A2AError.invalidResponse((response as? HTTPURLResponse)?.statusCode ?? 0)
        }
        registered = false
        heartbeatTask?.cancel()
        heartbeatTask = nil
    }

    // MARK: - Heartbeat

    func startHeartbeat(interval: TimeInterval = 30) {
        heartbeatTask?.cancel()
        heartbeatTask = Task {
            while !Task.isCancelled {
                do {
                    try await heartbeat()
                } catch {
                    // Silent heartbeat failure; server will mark offline if too many missed
                }
                try? await Task.sleep(nanoseconds: UInt64(interval * 1_000_000_000))
            }
        }
    }

    func stopHeartbeat() {
        heartbeatTask?.cancel()
        heartbeatTask = nil
    }

    private static let dateFormatter: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    func heartbeat() async throws {
        guard registered else { throw A2AError.notRegistered }
        let url = serverURL.appendingPathComponent("a2a/heartbeat")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let payload = HeartbeatPayload(
            agentId: agentCard.id,
            timestamp: Self.dateFormatter.string(from: Date())
        )
        request.httpBody = try encoder.encode(payload)

        let (_, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode) else {
            throw A2AError.invalidResponse((response as? HTTPURLResponse)?.statusCode ?? 0)
        }
    }

    // MARK: - Agent Discovery

    func listAgents() async throws -> [AgentCard] {
        let url = serverURL.appendingPathComponent("a2a/agents")
        let (data, response) = try await session.data(from: url)
        guard let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode) else {
            throw A2AError.invalidResponse((response as? HTTPURLResponse)?.statusCode ?? 0)
        }
        return try decoder.decode([AgentCard].self, from: data)
    }

    // MARK: - Messaging

    func sendMessage(_ message: A2AMessage) async throws {
        guard registered else { throw A2AError.notRegistered }
        let url = serverURL.appendingPathComponent("a2a/message")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try encoder.encode(message)

        let (_, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode) else {
            throw A2AError.invalidResponse((response as? HTTPURLResponse)?.statusCode ?? 0)
        }
    }

    func assignTask(_ task: AgentTask, to agent: AgentId) async throws {
        guard registered else { throw A2AError.notRegistered }
        let url = serverURL.appendingPathComponent("a2a/task/assign")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let payload = TaskAssignPayload(task: task, agentId: agent)
        request.httpBody = try encoder.encode(payload)

        let (_, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode) else {
            throw A2AError.invalidResponse((response as? HTTPURLResponse)?.statusCode ?? 0)
        }
    }

    func updateTaskState(id: UUID, state: AgentTaskState) async throws {
        guard registered else { throw A2AError.notRegistered }
        let url = serverURL.appendingPathComponent("a2a/task/update")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let payload = TaskUpdatePayload(id: id, state: state)
        request.httpBody = try encoder.encode(payload)

        let (_, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode) else {
            throw A2AError.invalidResponse((response as? HTTPURLResponse)?.statusCode ?? 0)
        }
    }

    // MARK: - Inbound Message Stream

    func messageStream() async throws -> AsyncStream<A2AMessage> {
        guard registered else { throw A2AError.notRegistered }
        let url = serverURL.appendingPathComponent("a2a/stream")
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")

        let (bytes, response) = try await session.bytes(for: request)
        guard let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode) else {
            throw A2AError.invalidResponse((response as? HTTPURLResponse)?.statusCode ?? 0)
        }

        return AsyncStream<A2AMessage> { continuation in
            let task = Task {
                do {
                    for try await line in bytes.lines {
                        if let message = self.parseSSELine(line) {
                            continuation.yield(message)
                        }
                    }
                    continuation.finish()
                } catch {
                    continuation.finish()
                }
            }
            continuation.onTermination = { @Sendable _ in
                task.cancel()
            }
        }
    }

    nonisolated private func parseSSELine(_ line: String) -> A2AMessage? {
        guard line.hasPrefix("data: ") else { return nil }
        let json = String(line.dropFirst(6))
        guard let data = json.data(using: .utf8) else { return nil }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        decoder.dateDecodingStrategy = .iso8601
        return try? decoder.decode(A2AMessage.self, from: data)
    }
}

// MARK: - Payload Types

private struct HeartbeatPayload: Codable, Sendable {
    let agentId: AgentId
    let timestamp: String
}

private struct TaskAssignPayload: Codable, Sendable {
    let task: AgentTask
    let agentId: AgentId
}

private struct TaskUpdatePayload: Codable, Sendable {
    let id: UUID
    let state: AgentTaskState
}
