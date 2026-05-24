import Foundation

actor EventThrottler {
    private var lastUpdateTime: Date = Date.distantPast
    private let minInterval: TimeInterval = 0.033
    private var pendingWork: (() async -> Void)?

    func throttle(work: @escaping () async -> Void) async {
        let now = Date()
        let elapsed = now.timeIntervalSince(lastUpdateTime)

        if elapsed >= minInterval {
            lastUpdateTime = now
            pendingWork = nil
            await work()
        } else {
            pendingWork = work
            let delay = minInterval - elapsed
            do {
                try await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
            } catch {
                pendingWork = nil
                return
            }
            if let pending = pendingWork {
                pendingWork = nil
                lastUpdateTime = Date()
                await pending()
            }
        }
    }
}
