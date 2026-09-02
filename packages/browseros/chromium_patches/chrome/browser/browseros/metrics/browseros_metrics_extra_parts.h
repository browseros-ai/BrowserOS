diff --git a/chrome/browser/browseros/metrics/browseros_metrics_extra_parts.h b/chrome/browser/browseros/metrics/browseros_metrics_extra_parts.h
new file mode 100644
index 0000000000000000000000000000000000000000..d52bd48464830ec03565f3972679a37d6966d615
--- /dev/null
+++ b/chrome/browser/browseros/metrics/browseros_metrics_extra_parts.h
@@ -0,0 +1,18 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_EXTRA_PARTS_H_
+#define CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_EXTRA_PARTS_H_
+
+class ChromeBrowserMainParts;
+
+namespace browseros_metrics {
+
+// Adds the process-scoped metrics lifecycle to ChromeBrowserMainParts without
+// exposing its internal service to browser startup code.
+void AddBrowserOSMetricsExtraParts(ChromeBrowserMainParts* main_parts);
+
+}  // namespace browseros_metrics
+
+#endif  // CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_EXTRA_PARTS_H_
