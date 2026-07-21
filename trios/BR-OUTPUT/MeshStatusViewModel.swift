import Foundation
import SwiftUI

/// View model for the Mesh tab. Polls clade-meshd and drives mesh operations.
@MainActor
final class MeshStatusViewModel: ObservableObject {
    @Published var nodeId: UInt32 = 0
    @Published var neighbors: [MeshNeighbor] = []
    @Published var routes: [MeshRoute] = []
    @Published var sessions: [MeshSession] = []
    @Published var metrics: MeshMetrics = MeshMetrics(link_loss_to_reroute_ms: nil, node_off_to_reroute_ms: nil)
    @Published var isReachable = false
    @Published var lastError: String?
    @Published var isLoading = false

    private let healthURL: URL
    private let statusURL: URL
    private var pollTimer: Timer?
    private let decoder = JSONDecoder()

    init(healthURL: URL = URL(string: ProjectPaths.meshHealthURL)!,
         statusURL: URL = URL(string: ProjectPaths.meshStatusURL)!) {
        self.healthURL = healthURL
        self.statusURL = statusURL
    }

    func startPolling(interval: TimeInterval = 2.0) {
        pollTimer?.invalidate()
        pollTimer = Timer.scheduledTimer(withTimeInterval: interval, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in
                await self?.refresh()
            }
        }
        Task { @MainActor [weak self] in
            await self?.refresh()
        }
    }

    func stopPolling() {
        pollTimer?.invalidate()
        pollTimer = nil
    }

    func refresh() async {
        isLoading = true
        defer { isLoading = false }

        do {
            let (healthData, healthResponse) = try await URLSession.shared.data(from: healthURL)
            guard let http = healthResponse as? HTTPURLResponse, http.statusCode == 200 else {
                isReachable = false
                lastError = "mesh health check failed"
                return
            }
            let health = try decoder.decode(MeshHealth.self, from: healthData)
            nodeId = health.node_id
            isReachable = true
            lastError = nil
        } catch {
            isReachable = false
            lastError = error.localizedDescription
            return
        }

        do {
            let (data, response) = try await URLSession.shared.data(from: statusURL)
            guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
                lastError = "mesh status returned non-200"
                return
            }
            let status = try decoder.decode(MeshStatus.self, from: data)
            neighbors = status.neighbors
            routes = status.routes
            sessions = status.sessions
            metrics = status.metrics
            lastError = nil
        } catch {
            lastError = error.localizedDescription
        }
    }

    func observe(peer: UInt32, weHeard: Bool, theyHeard: Bool) async {
        await postJSON(path: "/observe", body: MeshObserveRequest(peer: peer, we_heard: weHeard, they_heard: theyHeard))
        await refresh()
    }

    func hello(peer: UInt32, seq: UInt32 = 1, heard: [UInt32] = []) async {
        await postJSON(path: "/hello", body: MeshHelloRequest(peer: peer, seq: seq, heard: heard))
        await refresh()
    }

    func seedPeer(_ peer: UInt32) async {
        await postJSON(path: "/seed-peer", body: MeshPeerRequest(peer: peer))
        await refresh()
    }

    func forceDead(_ peer: UInt32) async {
        await postJSON(path: "/force-dead", body: MeshPeerRequest(peer: peer))
        await refresh()
    }

    func linkLoss() async {
        await postEmpty(path: "/link-loss")
    }

    func reroute() async {
        await postEmpty(path: "/reroute")
        await refresh()
    }

    private func postJSON<T: Encodable>(path: String, body: T) async {
        guard let url = URL(string: path, relativeTo: statusURL)?.absoluteURL else { return }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try? JSONEncoder().encode(body)
        do {
            let (_, response) = try await URLSession.shared.data(for: request)
            if let http = response as? HTTPURLResponse, http.statusCode != 200 {
                lastError = "\(path) returned \(http.statusCode)"
            }
        } catch {
            lastError = error.localizedDescription
        }
    }

    private func postEmpty(path: String) async {
        guard let url = URL(string: path, relativeTo: statusURL)?.absoluteURL else { return }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        do {
            let (_, response) = try await URLSession.shared.data(for: request)
            if let http = response as? HTTPURLResponse, http.statusCode != 200 {
                lastError = "\(path) returned \(http.statusCode)"
            }
        } catch {
            lastError = error.localizedDescription
        }
    }
}
