diff --git a/chrome/browser/browseros/server/process_controller_impl.h b/chrome/browser/browseros/server/process_controller_impl.h
new file mode 100644
index 0000000000000..46f7d9359dcd7
--- /dev/null
+++ b/chrome/browser/browseros/server/process_controller_impl.h
@@ -0,0 +1,35 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_PROCESS_CONTROLLER_IMPL_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_PROCESS_CONTROLLER_IMPL_H_
+
+#include "chrome/browser/browseros/server/process_controller.h"
+
+namespace browseros {
+
+// Production implementation of ProcessController.
+// Uses base::LaunchProcess and server_utils functions for real OS operations.
+class ProcessControllerImpl : public ProcessController {
+ public:
+  ProcessControllerImpl();
+  ~ProcessControllerImpl() override;
+
+  ProcessControllerImpl(const ProcessControllerImpl&) = delete;
+  ProcessControllerImpl& operator=(const ProcessControllerImpl&) = delete;
+
+  // ProcessController implementation:
+  LaunchResult Launch(const ServerLaunchConfig& config) override;
+  void Terminate(base::Process* process, bool wait) override;
+  bool WaitForExitWithTimeout(base::Process* process,
+                              base::TimeDelta timeout,
+                              int* exit_code) override;
+  bool Exists(base::ProcessId pid) override;
+  std::optional<int64_t> GetCreationTime(base::ProcessId pid) override;
+  bool Kill(base::ProcessId pid, base::TimeDelta graceful_timeout) override;
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_PROCESS_CONTROLLER_IMPL_H_
