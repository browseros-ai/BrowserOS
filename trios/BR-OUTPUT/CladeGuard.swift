import Foundation
import CryptoKit

/// Monitors Sovereign and Canary health. Triggers binary rollback on corruption.
/// Modeled on SessionGuard — same actor isolation, timer-based probing.
@MainActor
final class CladeGuard: ObservableObject {
    @Published var sovereignHealthy = false
    @Published var canaryHealthy = false
    @Published var lastSnapshotPath: String?
    @Published var isRollingBack = false

    private var probeTimer: Timer?
    private let sovereignCheck: HealthCheckTransport
    private let canaryCheck: HealthCheckTransport
    private let snapshotDir: String
    private let maxSnapshots = 10

    init(
        sovereignHealthURL: URL? = nil,
        canaryHealthURL: URL? = nil,
        snapshotDir: String? = nil
    ) {
        let sovereignURL = sovereignHealthURL ?? URL(string: "http://127.0.0.1:9105/health")!
        let canaryURL = canaryHealthURL ?? URL(string: "http://127.0.0.1:9205/health")!
        self.sovereignCheck = HealthCheckTransport(healthURL: sovereignURL)
        self.canaryCheck = HealthCheckTransport(healthURL: canaryURL)
        self.snapshotDir = snapshotDir ?? "\(ProjectPaths.trinity)/snapshots"
    }

    func startMonitoring(interval: TimeInterval = 10) {
        probeTimer?.invalidate()
        probeTimer = Timer.scheduledTimer(withTimeInterval: interval, repeats: true) { [weak self] _ in
            Task { @MainActor in
                await self?.tick()
            }
        }
        Task { @MainActor in
            await tick()
        }
    }

    func stopMonitoring() {
        probeTimer?.invalidate()
        probeTimer = nil
    }

    private func tick() async {
        sovereignHealthy = await sovereignCheck.check()
        canaryHealthy = await canaryCheck.check()

        if !sovereignHealthy {
            NSLog("[CladeGuard] Sovereign unhealthy — triggering rollback")
            await triggerRollback()
        }
    }

    /// Saves a binary snapshot before any risky operation. Computes SHA-256 checksum.
    func snapshotCurrentBinary() async {
        let fm = FileManager.default
        let ts = ISO8601DateFormatter().string(from: Date()).replacingOccurrences(of: ":", with: "-")
        let cladeId = (try? String(contentsOfFile: "\(ProjectPaths.trinity)/state/clade.json", encoding: .utf8)) ?? "unknown"
        let snapshotName = "trios_app-\(ts)-\(cladeId)"
        let snapshotPath = "\(snapshotDir)/\(snapshotName)"

        let targets = [
            ProjectPaths.triosBinary,
            "\(ProjectPaths.appBundle)/Contents/MacOS/trios"
        ]

        for source in targets where fm.fileExists(atPath: source) {
            do {
                if !fm.fileExists(atPath: snapshotDir) {
                    try fm.createDirectory(atPath: snapshotDir, withIntermediateDirectories: true)
                }
                try fm.copyItem(atPath: source, toPath: snapshotPath)

                // Compute SHA-256
                let data = try Data(contentsOf: URL(fileURLWithPath: snapshotPath))
                let hash = SHA256.hash(data: data).compactMap { String(format: "%02x", $0) }.joined()
                let checksumPath = "\(snapshotPath).sha256"
                try hash.write(toFile: checksumPath, atomically: true, encoding: .utf8)

                lastSnapshotPath = snapshotPath
                NSLog("[CladeGuard] Snapshot saved: \(snapshotPath) (sha256=\(hash.prefix(16))...)")

                pruneOldSnapshots()
                return
            } catch {
                NSLog("[CladeGuard] Snapshot failed: \(error)")
            }
        }
    }

    /// Verifies SHA-256 checksum for a snapshot.
    func verifyChecksum(_ snapshotPath: String) -> Bool {
        let checksumPath = "\(snapshotPath).sha256"
        let fm = FileManager.default
        guard fm.fileExists(atPath: checksumPath),
              let stored = try? String(contentsOfFile: checksumPath, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines) else {
            NSLog("[CladeGuard] No checksum file for \(snapshotPath), skipping verification")
            return true // permissive if no checksum
        }
        guard let data = try? Data(contentsOf: URL(fileURLWithPath: snapshotPath)) else { return false }
        let computed = SHA256.hash(data: data).compactMap { String(format: "%02x", $0) }.joined()
        let valid = stored == computed
        if !valid {
            NSLog("[CladeGuard] CHECKSUM MISMATCH for \(snapshotPath): stored=\(stored.prefix(16))... computed=\(computed.prefix(16))...")
        }
        return valid
    }

    private func pruneOldSnapshots() {
        let fm = FileManager.default
        guard let files = try? fm.contentsOfDirectory(atPath: snapshotDir) else { return }
        let snapshots = files
            .filter { $0.hasPrefix("trios_app-") && !$0.hasSuffix(".sha256") }
            .sorted { $0 > $1 } // descending by timestamp (newest first)
        for old in snapshots.dropFirst(maxSnapshots) {
            try? fm.removeItem(atPath: "\(snapshotDir)/\(old)")
            try? fm.removeItem(atPath: "\(snapshotDir)/\(old).sha256")
        }
    }

    private func triggerRollback() async {
        guard !isRollingBack else {
            NSLog("[CladeGuard] Rollback already in progress")
            return
        }
        isRollingBack = true
        defer { isRollingBack = false }

        let fm = FileManager.default

        if let snapshot = lastSnapshotPath, fm.fileExists(atPath: snapshot), verifyChecksum(snapshot) {
            await applySnapshot(snapshot)
            return
        }

        guard let files = try? fm.contentsOfDirectory(atPath: snapshotDir) else {
            NSLog("[CladeGuard] No snapshots available — manual intervention required")
            return
        }
        let snapshots = files
            .filter { $0.hasPrefix("trios_app-") && !$0.hasSuffix(".sha256") }
            .sorted { $0 > $1 }
        guard let newest = snapshots.first else {
            NSLog("[CladeGuard] Snapshot directory empty — manual intervention required")
            return
        }
        let candidate = "\(snapshotDir)/\(newest)"
        guard verifyChecksum(candidate) else {
            NSLog("[CladeGuard] Newest snapshot checksum invalid — manual intervention required")
            return
        }
        await applySnapshot(candidate)
    }

    private func applySnapshot(_ snapshotPath: String) async {
        let fm = FileManager.default
        let targets = [
            ProjectPaths.triosBinary,
            "\(ProjectPaths.appBundle)/Contents/MacOS/trios"
        ]

        for target in targets {
            do {
                if fm.fileExists(atPath: target) {
                    try fm.removeItem(atPath: target)
                }
                try fm.copyItem(atPath: snapshotPath, toPath: target)
            } catch {
                NSLog("[CladeGuard] Failed to copy snapshot to \(target): \(error)")
            }
        }

        NSLog("[CladeGuard] Rolled back to \(snapshotPath)")

        // Restart the Sovereign app
        Task { @MainActor in
            let task = Process()
            task.executableURL = URL(fileURLWithPath: "/usr/bin/open")
            task.arguments = ["-n", ProjectPaths.appBundle]
            try? task.run()
        }

        // Boot Probe: wait then verify health
        try? await Task.sleep(nanoseconds: 10 * 1_000_000_000)
        let healthy = await sovereignCheck.check()
        if healthy {
            NSLog("[CladeGuard] Boot probe PASSED — Sovereign healthy after rollback")
        } else {
            NSLog("[CladeGuard] Boot probe FAILED — Sovereign still unhealthy after rollback")
        }
    }

    /// Manual emergency rollback — callable from CLI or skill.
    func emergencyRollback() async {
        NSLog("[CladeGuard] EMERGENCY rollback invoked")
        await triggerRollback()
    }

    /// Boot probe after promotion: verifies Sovereign health within timeout.
    func bootProbe(timeoutSeconds: UInt64 = 15) async -> Bool {
        let deadline = Date().addingTimeInterval(TimeInterval(timeoutSeconds))
        while Date() < deadline {
            let healthy = await sovereignCheck.check()
            if healthy {
                NSLog("[CladeGuard] Boot probe PASSED")
                return true
            }
            try? await Task.sleep(nanoseconds: 1 * 1_000_000_000)
        }
        NSLog("[CladeGuard] Boot probe FAILED — reverting to last snapshot")
        await triggerRollback()
        return false
    }
}
