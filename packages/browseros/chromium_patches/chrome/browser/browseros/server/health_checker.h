diff --git a/chrome/browser/browseros/server/health_checker.h b/chrome/browser/browseros/server/health_checker.h
new file mode 100644
index 0000000000000..e4a9600866374
--- /dev/null
+++ b/chrome/browser/browseros/server/health_checker.h
@@ -0,0 +1,26 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_HEALTH_CHECKER_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_HEALTH_CHECKER_H_
+
+#include "base/functional/callback.h"
+
+namespace browseros {
+
+// Interface for HTTP health check probes.
+// Abstracted to enable unit testing without real network requests.
+class HealthChecker {
+ public:
+  virtual ~HealthChecker() = default;
+
+  // Perform async health check by querying the /health endpoint.
+  // Invokes callback with true on HTTP 200, false otherwise.
+  virtual void CheckHealth(int port,
+                           base::OnceCallback<void(bool success)> callback) = 0;
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_HEALTH_CHECKER_H_
