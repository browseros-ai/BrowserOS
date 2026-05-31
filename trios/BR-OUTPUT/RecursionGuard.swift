import Cocoa
import Foundation

/// Prevents recursive self-launch of trios by enforcing a single running instance.
/// Uses three methods in priority order:
///   1. POSIX file lock (works for ALL launch paths including ./trios_app)
///   2. NSRunningApplication by bundle ID (works for .app bundles)
///   3. PID file fallback (works for direct binary ./trios_app)
///
/// SAFETY: The file lock is held for the lifetime of the process. Closing the FD
/// or process termination automatically releases it.
final class RecursionGuard {
    static let shared = RecursionGuard()

    private let lockFilePath = "/tmp/trios_singleton.lock"
    private let pidFilePath  = "/tmp/trios_singleton.pid"
    private let bundleID = "com.browseros.trios"

    /// File descriptor for the POSIX lock. Must stay open for the lock to persist.
    private var lockFD: Int32 = -1

    deinit {
        cleanup()
    }

    /// Returns true if this instance should proceed; false if another is already running.
    @discardableResult
    func ensureSingleInstance() -> Bool {
        // Method 1: POSIX advisory file lock (fastest, most reliable)
        if !acquireFileLock() {
            NSLog("[RecursionGuard] Blocked: another instance holds the POSIX lock")
            activateExistingInstance()
            return false
        }

        // Method 2: NSRunningApplication by bundle ID (for .app bundles)
        let running = NSRunningApplication.runningApplications(withBundleIdentifier: bundleID)
        let current = NSRunningApplication.current
        let others = running.filter { $0 != current }
        if !others.isEmpty {
            NSLog("[RecursionGuard] Blocked: another trios instance detected via bundle ID (PIDs: \(others.map { $0.processIdentifier }))")
            releaseFileLock()
            others.first?.activate()
            return false
        }

        // Method 3: PID file fallback (cleanup stale entries)
        if let pid = readPIDFile(), pid != getpid() {
            if isTriosProcess(pid: pid) {
                NSLog("[RecursionGuard] Blocked: another trios instance detected via PID file (PID: \(pid))")
                releaseFileLock()
                activateProcess(pid: pid)
                return false
            }
            // Stale PID file — clean it up
            try? FileManager.default.removeItem(atPath: pidFilePath)
        }

        // Write our PID so future launches see us
        writePIDFile()
        return true
    }

    /// Removes the PID file on graceful exit. Releases the file lock.
    func cleanup() {
        let currentPID = getpid()
        if let stored = readPIDFile(), stored == currentPID {
            try? FileManager.default.removeItem(atPath: pidFilePath)
        }
        releaseFileLock()
    }

    // MARK: - POSIX File Lock

    /// Acquires an exclusive (write) lock on the lock file.
    /// The lock persists as long as the file descriptor remains open.
    private func acquireFileLock() -> Bool {
        let fd = open(lockFilePath, O_CREAT | O_RDWR, 0o666)
        guard fd >= 0 else {
            NSLog("[RecursionGuard] Failed to open lock file: \(errno)")
            return false
        }

        var lock = flock()
        lock.l_type   = Int16(F_WRLCK)
        lock.l_whence = Int16(SEEK_SET)
        lock.l_start  = 0
        lock.l_len    = 0
        lock.l_pid    = 0

        let result = fcntl(fd, F_SETLK, &lock)
        if result == -1 {
            close(fd)
            return false
        }

        lockFD = fd
        return true
    }

    /// Releases the file lock and closes the descriptor.
    private func releaseFileLock() {
        guard lockFD >= 0 else { return }

        var lock = flock()
        lock.l_type   = Int16(F_UNLCK)
        lock.l_whence = Int16(SEEK_SET)
        lock.l_start  = 0
        lock.l_len    = 0
        lock.l_pid    = 0

        _ = fcntl(lockFD, F_SETLK, &lock)
        close(lockFD)
        lockFD = -1
    }

    // MARK: - PID File

    private func readPIDFile() -> pid_t? {
        guard let content = try? String(contentsOfFile: pidFilePath, encoding: .utf8),
              let pid = Int32(content.trimmingCharacters(in: .whitespacesAndNewlines)) else {
            return nil
        }
        return pid
    }

    private func writePIDFile() {
        let pid = getpid()
        do {
            try String(pid).write(toFile: pidFilePath, atomically: true, encoding: .utf8)
        } catch {
            NSLog("[RecursionGuard] Failed to write PID file: \(error)")
        }
    }

    // MARK: - Process Detection

    private func isTriosProcess(pid: pid_t) -> Bool {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/bin/ps")
        task.arguments = ["-p", String(pid), "-o", "comm="]
        let pipe = Pipe()
        task.standardOutput = pipe
        do {
            try task.run()
            task.waitUntilExit()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            if let name = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) {
                return name.contains("trios") || name.contains("trios_app")
            }
        } catch {
            return false
        }
        return false
    }

    // MARK: - Activation

    private func activateExistingInstance() {
        if let app = NSRunningApplication.runningApplications(withBundleIdentifier: bundleID).first {
            app.activate()
            return
        }
        // Fallback to PID-based activation
        if let pid = readPIDFile() {
            activateProcess(pid: pid)
        }
    }

    private func activateProcess(pid: pid_t) {
        if let app = NSRunningApplication.runningApplications(withBundleIdentifier: bundleID)
            .first(where: { $0.processIdentifier == pid }) {
            app.activate()
            return
        }
        let script = """
        tell application "System Events"
            set frontmost of first process whose unix id is \(pid) to true
        end tell
        """
        var errorInfo: NSDictionary?
        if let appleScript = NSAppleScript(source: script) {
            appleScript.executeAndReturnError(&errorInfo)
        }
    }
}
