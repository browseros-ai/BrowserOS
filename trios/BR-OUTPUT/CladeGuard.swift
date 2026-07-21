import Foundation
import CryptoKit

/// Monitors Sovereign and Canary health. Triggers binary rollback on corruption.
/// Modeled on SessionGuard - same actor isolation, timer-based probing.
///
/// SAFETY: Never auto-restarts the app. Rollback replaces the binary on disk;
/// the user must restart manually. This prevents recursive launch cascades.
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

    // SAFETY: Grace period + consecutive-failure threshold prevents
    // immediate rollback on cold-start before the server is ready.
    private let bootTimestamp = Date()
    private let bootGracePeriod: TimeInterval = 60
    private let maxConsecutiveFailures = 3
    private let rollbackCooldown: TimeInterval = 30
    private var consecutiveFailures = 0
    private var lastRollbackTime: Date?

    init(
        sovereignHealthURL: URL? = nil,
        canaryHealthURL: URL? = nil,
        snapshotDir: String? = nil
    ) {
        let sovereignURL = sovereignHealthURL ?? URL(string: ProjectPaths.browserOSHealthURL)
            ?? URL(fileURLWithPath: "/dev/null")
        let canaryURL = canaryHealthURL ?? URL(string: ProjectPaths.canaryHealthURL)
            ?? URL(fileURLWithPath: "/dev/null")
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

        if sovereignHealthy {
            consecutiveFailures = 0
            return
        }

        consecutiveFailures += 1
        NSLog("[CladeGuard] Sovereign unhealthy (consecutive failures: \(consecutiveFailures))")

        // During boot grace period we only log; we do NOT roll back.
        let elapsed = Date().timeIntervalSince(bootTimestamp)
        guard elapsed > bootGracePeriod else {
            NSLog("[CladeGuard] Within boot grace period (\(Int(elapsed))s < \(Int(bootGracePeriod))s) - skipping rollback")
            return
        }

        guard consecutiveFailures >= maxConsecutiveFailures else {
            NSLog("[CladeGuard] Failure count \(consecutiveFailures) < threshold \(maxConsecutiveFailures) - waiting")
            return
        }

        // Cooldown: do not roll back more than once every N seconds.
        if let last = lastRollbackTime, Date().timeIntervalSince(last) < rollbackCooldown {
            NSLog("[CladeGuard] Rollback cooldown active - skipping")
            return
        }

        NSLog("[CladeGuard] Sovereign unhealthy - triggering rollback")
        await triggerRollback()
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
    /// Returns false if the sidecar is missing or does not match.
    func verifyChecksum(_ snapshotPath: String) -> Bool {
        let checksumPath = "\(snapshotPath).sha256"
        let fm = FileManager.default
        guard fm.fileExists(atPath: checksumPath),
              let stored = try? String(contentsOfFile: checksumPath, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines) else {
            NSLog("[CladeGuard] No checksum file for \(snapshotPath) - rejecting snapshot")
            return false
        }
        guard let data = try? Data(contentsOf: URL(fileURLWithPath: snapshotPath)) else {
            NSLog("[CladeGuard] Cannot read snapshot \(snapshotPath) for checksum verification")
            return false
        }
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
            do {
                try fm.removeItem(atPath: "\(snapshotDir)/\(old)")
            } catch {
                NSLog("[CladeGuard] Failed to prune snapshot \(old): \(error)")
            }
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

        lastRollbackTime = Date()

        let fm = FileManager.default

        if let snapshot = lastSnapshotPath, fm.fileExists(atPath: snapshot), verifyChecksum(snapshot) {
            await applySnapshot(snapshot)
            return
        }

        guard let files = try? fm.contentsOfDirectory(atPath: snapshotDir) else {
            NSLog("[CladeGuard] No snapshots available - manual intervention required")
            return
        }
        let snapshots = files
            .filter { $0.hasPrefix("trios_app-") && !$0.hasSuffix(".sha256") }
            .sorted { $0 > $1 }
        guard let newest = snapshots.first else {
            NSLog("[CladeGuard] Snapshot directory empty - manual intervention required")
            return
        }
        let candidate = "\(snapshotDir)/\(newest)"
        guard verifyChecksum(candidate) else {
            NSLog("[CladeGuard] Newest snapshot checksum invalid - manual intervention required")
            return
        }
        await applySnapshot(candidate)
    }

    /// Replaces the on-disk binary with the chosen snapshot atomically.
    /// Does NOT kill or relaunch the running process - that is the user's decision.
    private func applySnapshot(_ snapshotPath: String) async {
        let fm = FileManager.default
        let targets = [
            ProjectPaths.triosBinary,
            "\(ProjectPaths.appBundle)/Contents/MacOS/trios"
        ]

        for target in targets {
            do {
                guard fm.fileExists(atPath: snapshotPath) else {
                    NSLog("[CladeGuard] Snapshot vanished before copy: \(snapshotPath)")
                    continue
                }

                let targetURL = URL(fileURLWithPath: target)
                let snapshotURL = URL(fileURLWithPath: snapshotPath)
                let tempURL = targetURL.appendingPathExtension("tmp")

                // Copy snapshot to a temp file next to the target, then atomically swap.
                if fm.fileExists(atPath: tempURL.path) {
                    try fm.removeItem(at: tempURL)
                }
                try fm.copyItem(at: snapshotURL, to: tempURL)

                let coordinator = NSFileCoordinator(filePresenter: nil)
                var coordinatorError: NSError?
                var replacementError: Error?
                coordinator.coordinate(writingItemAt: targetURL, options: .forReplacing, writingItemAt: tempURL, options: [], error: &coordinatorError) { writeURL, tempWriteURL in
                    do {
                        _ = try fm.replaceItemAt(writeURL, withItemAt: tempWriteURL)
                    } catch {
                        replacementError = error
                    }
                }
                if let error = replacementError ?? coordinatorError {
                    throw error
                }
            } catch {
                NSLog("[CladeGuard] Failed to atomically replace \(target) with snapshot: \(error)")
            }
        }

        NSLog("[CladeGuard] Rolled back to \(snapshotPath)")
        NSLog("[CladeGuard] WARNING: The binary on disk has been restored. Please restart trios manually to run the restored version.")

        // Boot Probe: verify the restored binary would work (health via current runtime)
        try? await Task.sleep(nanoseconds: 2 * 1_000_000_000)
        let healthy = await sovereignCheck.check()
        if healthy {
            NSLog("[CladeGuard] Boot probe PASSED - Sovereign healthy")
        } else {
            NSLog("[CladeGuard] Boot probe FAILED - Sovereign still unhealthy after rollback. Manual restart recommended.")
        }
    }

    /// Manual emergency rollback - callable from CLI or skill.
    func emergencyRollback() async {
        NSLog("[CladeGuard] EMERGENCY rollback invoked")
        await triggerRollback()
    }

    /// Boot probe after promotion: verifies Sovereign health within timeout.
    /// Returns true if healthy; does NOT auto-rollback here -
    /// the caller decides whether to revert.
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
        NSLog("[CladeGuard] Boot probe FAILED - manual rollback may be needed")
        return false
    }
}
