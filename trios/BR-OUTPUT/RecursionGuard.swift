import Cocoa
import Foundation

/// Prevents recursive self-launch of trios by enforcing a single running instance.
/// Spec source: `.trinity/specs/recursion-guard.md`
///
/// Uses three methods in priority order:
///   1. POSIX file lock (works for ALL launch paths including ./trios_app)
///   2. NSRunningApplication by bundle ID (works for .app bundles)
///   3. PID file fallback (works for direct binary ./trios_app)
///
/// SAFETY: The file lock is held for the lifetime of the process. Closing the FD
/// or process termination automatically releases it. To minimize the stale-PID
/// race window we write the PID file immediately after acquiring the lock and
/// verify a competing PID by both `comm` and command-line arguments.
final class RecursionGuard {
    static let shared = RecursionGuard()

    private let lockFilePath = ProjectPaths.singletonLockFile
    private let pidFilePath  = ProjectPaths.singletonPIDFile
    private let bundleID     = ProjectPaths.bundleIdentifier

    /// File descriptor for the POSIX lock. Must stay open for the lock to persist.
    private var lockFD: Int32 = -1

    deinit {
        cleanup()
    }

    /// Returns true if this instance should proceed; false if another is already running.
    /// When false, the existing instance is brought to the foreground.
    @discardableResult
    func ensureSingleInstance() -> Bool {
        // Method 1: POSIX advisory file lock with retries. The lock is the most
        // reliable signal because it survives .app/bare-binary differences.
        if !acquireFileLock(retries: 5, delayMicroseconds: 10_000) {
            NSLog("[RecursionGuard] Blocked: another instance holds the POSIX lock")
            activateExistingInstance()
            return false
        }

        // Write our PID immediately so any concurrently starting peer sees us
        // before it finishes its own PID-file check.
        writePIDFile()

        // Method 2: NSRunningApplication by bundle ID (for .app bundles). This
        // also lets macOS activate the existing instance instead of launching a
        // new process when `open trios.app` is invoked.
        let running = NSRunningApplication.runningApplications(withBundleIdentifier: bundleID)
        let current = NSRunningApplication.current
        let others = running.filter { $0 != current }
        if !others.isEmpty {
            NSLog("[RecursionGuard] Blocked: another trios instance detected via bundle ID (PIDs: \(others.map { $0.processIdentifier }))")
            releaseFileLock()
            others.first?.activate()
            return false
        }

        // Method 3: PID file fallback (cleanup stale entries). A competing PID
        // must actually be a trios process; otherwise we treat the file as stale.
        if let pid = readPIDFile(), pid != getpid() {
            if isTriosProcess(pid: pid) {
                NSLog("[RecursionGuard] Blocked: another trios instance detected via PID file (PID: \(pid))")
                releaseFileLock()
                activateProcess(pid: pid)
                return false
            }
            // Stale PID file - clean it up
            try? FileManager.default.removeItem(atPath: pidFilePath)
        }

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

    /// Acquires an exclusive (write) lock on the lock file, retrying briefly.
    /// The lock persists as long as the file descriptor remains open.
    private func acquireFileLock(retries: Int, delayMicroseconds: UInt32) -> Bool {
        for attempt in 0...retries {
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
            if result == 0 {
                lockFD = fd
                return true
            }
            close(fd)

            if attempt < retries {
                usleep(delayMicroseconds)
            }
        }
        return false
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

    /// Returns true if `pid` is a trios/trios_app process. We check both the
    /// short command name (`comm`) and the full command-line arguments so a
    /// stale PID pointing at an unrelated process is not mistaken for trios.
    private func isTriosProcess(pid: pid_t) -> Bool {
        guard pid > 0 else { return false }

        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/bin/ps")
        task.arguments = ["-p", String(pid), "-o", "comm=,args="]
        let pipe = Pipe()
        task.standardOutput = pipe
        do {
            try task.run()
            task.waitUntilExit()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            guard let line = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !line.isEmpty else {
                return false
            }
            let tokens = line.split(separator: " ", omittingEmptySubsequences: true).map { String($0) }
            guard let comm = tokens.first else { return false }
            let isTriosComm = comm == "trios" || comm == "trios_app" || comm.contains("trios")
            let isTriosArgs = tokens.contains {
                $0.hasSuffix("/trios.app/Contents/MacOS/trios") ||
                $0 == "./trios_app" ||
                $0 == "trios_app"
            }
            return isTriosComm || isTriosArgs
        } catch {
            return false
        }
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
