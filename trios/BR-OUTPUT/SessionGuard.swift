import Foundation

/// Monitors and maintains the longevity of Trios chat sessions.
/// Handles reconnection for SSE transport and A2A registration.
@MainActor
final class SessionGuard: ObservableObject {
    @Published var isSessionActive = false
    @Published var lastReconnectAttempt: Date?
    @Published var reconnectCount = 0

    private let healthCheck: ChatHealthCheckProtocol
    private let a2aClient: A2ARegistryClient?
    private var healthTimer: Timer?
    private var reconnectTask: Task<Void, Never>?
    private let maxReconnectAttempts = 5
    private let reconnectDelay: TimeInterval = 5

    init(healthCheck: ChatHealthCheckProtocol, a2aClient: A2ARegistryClient? = nil) {
        self.healthCheck = healthCheck
        self.a2aClient = a2aClient
    }

    func startMonitoring() {
        healthTimer?.invalidate()
        healthTimer = Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { [weak self] _ in
            Task { @MainActor in
                await self?.performHealthCheck()
            }
        }
        // Immediate first check
        Task {
            await performHealthCheck()
        }
    }

    func stopMonitoring() {
        healthTimer?.invalidate()
        healthTimer = nil
        reconnectTask?.cancel()
        reconnectTask = nil
    }

    private func performHealthCheck() async {
        let healthy = await healthCheck.check()
        isSessionActive = healthy
        if !healthy {
            await attemptReconnection()
        }
    }

    private func attemptReconnection() async {
        guard reconnectCount < maxReconnectAttempts else {
            NSLog("[SessionGuard] Max reconnect attempts reached, giving up")
            return
        }
        reconnectCount += 1
        lastReconnectAttempt = Date()
        NSLog("[SessionGuard] Reconnection attempt \(reconnectCount)/\(maxReconnectAttempts)")

        // Wait with exponential backoff
        let delay = reconnectDelay * pow(2.0, Double(reconnectCount - 1))
        try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))

        // Check health again after delay
        let healthy = await healthCheck.check()
        if healthy {
            NSLog("[SessionGuard] Health restored")
            reconnectCount = 0
            isSessionActive = true
            // Re-register A2A if needed
            if let client = a2aClient {
                do {
                    try await client.register()
                } catch {
                    NSLog("[SessionGuard] A2A re-registration failed: \(error)")
                }
                await client.startHeartbeat(interval: 30)
            }
        } else {
            NSLog("[SessionGuard] Still unhealthy after reconnection attempt")
        }
    }

    func resetReconnectCount() {
        reconnectCount = 0
    }
}
