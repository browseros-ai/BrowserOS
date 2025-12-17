diff --git a/chrome/browser/browseros/metrics/browseros_metrics.h b/chrome/browser/browseros/metrics/browseros_metrics.h
new file mode 100644
index 0000000000000..6da70921b619a
--- /dev/null
+++ b/chrome/browser/browseros/metrics/browseros_metrics.h
@@ -0,0 +1,40 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_H_
+#define CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_H_
+
+#include <string>
+#include <utility>
+
+#include "base/values.h"
+
+namespace browseros_metrics {
+
+// Simple static API for logging BrowserOS metrics.
+// Usage: BrowserOSMetrics::Log("event.name");
+class BrowserOSMetrics {
+ public:
+  // Log an event with no properties
+  // sample_rate: 0.0 to 1.0, defaults to 1.0 (always log)
+  // For example, sample_rate=0.1 means log only 10% of the time
+  static void Log(const std::string& event_name, double sample_rate = 1.0);
+
+  // Log an event with properties using initializer list
+  // Example: Log("event", {{"key1", "value1"}, {"key2", 123}})
+  static void Log(const std::string& event_name,
+                  std::initializer_list<std::pair<std::string, base::Value>> properties,
+                  double sample_rate = 1.0);
+
+  // Log an event with pre-built properties dict
+  static void Log(const std::string& event_name, base::Value::Dict properties,
+                  double sample_rate = 1.0);
+
+ private:
+  BrowserOSMetrics() = delete;
+};
+
+}  // namespace browseros_metrics
+
+#endif  // CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_H_
