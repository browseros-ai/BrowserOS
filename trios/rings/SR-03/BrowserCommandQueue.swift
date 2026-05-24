import Foundation

actor BrowserCommandQueue {
    private var queue: [BrowserCommand] = []
    private var pending: [UUID: CheckedContinuation<BrowserResult, Never>] = [:]
    private var results: [UUID: BrowserResult] = [:]

    func enqueue(_ command: BrowserCommand) async {
        queue.append(command)
    }

    func dequeue() async -> BrowserCommand? {
        guard !queue.isEmpty else { return nil }
        return queue.removeFirst()
    }

    func awaitResult(for commandId: UUID) async -> BrowserResult {
        if let result = results.removeValue(forKey: commandId) {
            return result
        }
        return await withCheckedContinuation { continuation in
            pending[commandId] = continuation
        }
    }

    func deliver(_ result: BrowserResult) async {
        if let continuation = pending.removeValue(forKey: result.commandId) {
            continuation.resume(returning: result)
        } else {
            results[result.commandId] = result
        }
    }
}
