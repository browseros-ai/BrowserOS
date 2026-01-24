diff --git a/chrome/browser/browseros/server/browseros_server_utils.h b/chrome/browser/browseros/server/browseros_server_utils.h
new file mode 100644
index 0000000000000..ca70bb0909aa3
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_utils.h
@@ -0,0 +1,84 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_UTILS_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_UTILS_H_
+
+#include <optional>
+#include <set>
+#include <string>
+
+#include "base/files/file_path.h"
+#include "base/process/process_handle.h"
+#include "base/time/time.h"
+
+namespace browseros::server_utils {
+
+// =============================================================================
+// Port Utilities
+// =============================================================================
+
+// Finds an available port starting from starting_port.
+// Skips ports in the excluded set to prevent collisions between services.
+int FindAvailablePort(int starting_port, const std::set<int>& excluded);
+
+// Returns true if the specified port is available for binding.
+bool IsPortAvailable(int port);
+
+// =============================================================================
+// Path Utilities
+// =============================================================================
+
+// Returns the execution directory under user data (~/.browseros or equivalent).
+// Creates the directory if it doesn't exist.
+base::FilePath GetExecutionDir();
+
+// Returns path to the bundled server executable.
+base::FilePath GetBundledExecutablePath();
+
+// Returns path to the bundled server resources directory.
+base::FilePath GetBundledResourcesPath();
+
+// Returns path to the lock file (execution_dir/server.lock).
+base::FilePath GetLockFilePath();
+
+// Returns path to the state file (execution_dir/server.state).
+base::FilePath GetStateFilePath();
+
+// =============================================================================
+// State File (Orphan Recovery)
+// =============================================================================
+
+struct ServerState {
+  base::ProcessId pid = 0;
+  int64_t creation_time = 0;  // Process creation time in milliseconds
+};
+
+// Reads the state file. Returns nullopt if file doesn't exist or is invalid.
+std::optional<ServerState> ReadStateFile();
+
+// Writes the state file with pid and creation_time.
+bool WriteStateFile(const ServerState& state);
+
+// Deletes the state file.
+bool DeleteStateFile();
+
+// =============================================================================
+// Process Utilities
+// =============================================================================
+
+// Returns the process creation time in milliseconds since epoch.
+// Platform-specific implementation (macOS/Linux/Windows).
+std::optional<int64_t> GetProcessCreationTime(base::ProcessId pid);
+
+// Returns true if a process with the given PID exists.
+bool ProcessExists(base::ProcessId pid);
+
+// Kills a process. First sends SIGTERM, waits for graceful_timeout,
+// then sends SIGKILL if still running.
+bool KillProcess(base::ProcessId pid, base::TimeDelta graceful_timeout);
+
+}  // namespace browseros::server_utils
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_UTILS_H_
