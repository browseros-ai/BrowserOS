diff --git a/chrome/browser/browseros/server/process_controller.h b/chrome/browser/browseros/server/process_controller.h
new file mode 100644
index 0000000000000..9a8b36135f878
--- /dev/null
+++ b/chrome/browser/browseros/server/process_controller.h
@@ -0,0 +1,53 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_PROCESS_CONTROLLER_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_PROCESS_CONTROLLER_H_
+
+#include <optional>
+
+#include "base/functional/callback.h"
+#include "base/process/process.h"
+#include "base/time/time.h"
+#include "chrome/browser/browseros/server/browseros_server_config.h"
+
+namespace browseros {
+
+struct LaunchResult {
+  base::Process process;
+  bool used_fallback = false;
+};
+
+// Interface for process lifecycle operations.
+// Abstracted to enable unit testing without spawning real OS processes.
+class ProcessController {
+ public:
+  virtual ~ProcessController() = default;
+
+  // Launch server process with the given configuration.
+  // Returns LaunchResult with the process handle (invalid if launch failed)
+  // and whether the fallback binary was used.
+  virtual LaunchResult Launch(const ServerLaunchConfig& config) = 0;
+
+  // Terminate a running process.
+  // If wait=true, blocks until process exits (must be called from background
+  // thread). If wait=false, just sends kill signal and returns immediately.
+  virtual void Terminate(base::Process* process, bool wait) = 0;
+
+  // Check if a process with the given PID exists.
+  virtual bool Exists(base::ProcessId pid) = 0;
+
+  // Get process creation time in milliseconds since epoch.
+  // Returns nullopt if process doesn't exist or time couldn't be retrieved.
+  virtual std::optional<int64_t> GetCreationTime(base::ProcessId pid) = 0;
+
+  // Kill process with graceful timeout.
+  // First sends SIGTERM, waits for graceful_timeout, then SIGKILL if needed.
+  // Returns true if process was successfully killed (or already gone).
+  virtual bool Kill(base::ProcessId pid, base::TimeDelta graceful_timeout) = 0;
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_PROCESS_CONTROLLER_H_
