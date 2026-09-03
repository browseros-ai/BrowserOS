diff --git a/chrome/browser/browseros/metrics/browseros_metrics.h b/chrome/browser/browseros/metrics/browseros_metrics.h
new file mode 100644
index 0000000000000000000000000000000000000000..7f698a80fed05ded22a535fed8001c9c05b8a3a4
--- /dev/null
+++ b/chrome/browser/browseros/metrics/browseros_metrics.h
@@ -0,0 +1,31 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_H_
+#define CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_H_
+
+#include <string_view>
+
+#include "base/values.h"
+
+namespace browseros_metrics {
+
+// Thread-safe entry point for native product analytics. Process lifecycle,
+// product routing, installation identity, and delivery stay behind this API so
+// callers never need profile or preference access.
+class BrowserOSMetrics {
+ public:
+  // Safe from any thread. Sampling occurs before the event is handed to the UI
+  // sequence that owns the process-wide reporter.
+  static void Log(std::string_view event_name,
+                  base::DictValue properties = {},
+                  double sample_rate = 1.0);
+
+ private:
+  BrowserOSMetrics() = delete;
+};
+
+}  // namespace browseros_metrics
+
+#endif  // CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_H_
