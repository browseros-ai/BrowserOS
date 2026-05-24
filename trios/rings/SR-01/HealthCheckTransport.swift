import Foundation

actor HealthCheckTransport: ChatHealthCheckProtocol {
    private let healthURL: URL

    init(healthURL: URL = URL(string: "http://127.0.0.1:9105/health")!) {
        self.healthURL = healthURL
    }

    func check() async -> Bool {
        do {
            let (_, response) = try await URLSession.shared.data(from: healthURL)
            guard let httpResponse = response as? HTTPURLResponse else { return false }
            return (200...299).contains(httpResponse.statusCode)
        } catch {
            return false
        }
    }
}
