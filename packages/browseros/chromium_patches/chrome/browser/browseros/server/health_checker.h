diff --git a/chrome/browser/browseros/server/health_checker.h b/chrome/browser/browseros/server/health_checker.h
new file mode 100644
index 0000000000000..7f4516c788d3a
--- /dev/null
+++ b/chrome/browser/browseros/server/health_checker.h
@@ -0,0 +1,25 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_HEALTH_CHECKER_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_HEALTH_CHECKER_H_
+
+#include <string>
+
+#include "base/functional/callback.h"
+
+namespace browseros {
+
+class HealthChecker {
+ public:
+  virtual ~HealthChecker() = default;
+
+  virtual void CheckHealth(int port,
+                           const std::string& path,
+                           base::OnceCallback<void(bool success)> callback) = 0;
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_HEALTH_CHECKER_H_
