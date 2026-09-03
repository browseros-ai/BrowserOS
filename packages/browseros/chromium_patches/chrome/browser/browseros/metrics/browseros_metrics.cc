diff --git a/chrome/browser/browseros/metrics/browseros_metrics.cc b/chrome/browser/browseros/metrics/browseros_metrics.cc
new file mode 100644
index 0000000000000000000000000000000000000000..209392c1f22fe57c7e4a3f232007cad012997ee2
--- /dev/null
+++ b/chrome/browser/browseros/metrics/browseros_metrics.cc
@@ -0,0 +1,61 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/metrics/browseros_metrics.h"
+
+#include <string>
+#include <string_view>
+#include <utility>
+
+#include "base/functional/bind.h"
+#include "base/logging.h"
+#include "base/rand_util.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics_service.h"
+#include "content/public/browser/browser_task_traits.h"
+#include "content/public/browser/browser_thread.h"
+
+namespace browseros_metrics {
+
+namespace {
+
+void LogOnUIThread(std::string event_name, base::DictValue properties) {
+  BrowserOSMetricsService* service = GetBrowserOSMetricsServiceForProcess();
+  if (service) {
+    service->CaptureEvent(std::move(event_name), std::move(properties));
+  } else {
+    VLOG(1) << "browseros: Metrics reporter is not available for event: "
+            << event_name;
+  }
+}
+
+}  // namespace
+
+// static
+void BrowserOSMetrics::Log(std::string_view event_name,
+                           base::DictValue properties,
+                           double sample_rate) {
+  if (sample_rate <= 0.0 || sample_rate > 1.0) {
+    return;
+  }
+
+  if (sample_rate < 1.0) {
+    if (base::RandDouble() > sample_rate) {
+      return;
+    }
+    properties.Set("sample_rate", sample_rate);
+  }
+
+  std::string owned_event_name(event_name);
+  if (content::BrowserThread::CurrentlyOn(content::BrowserThread::UI)) {
+    LogOnUIThread(std::move(owned_event_name), std::move(properties));
+  } else {
+    // The service has UI-sequence affinity; cross-thread callers transfer
+    // ownership of the complete event before returning.
+    content::GetUIThreadTaskRunner({})->PostTask(
+        FROM_HERE, base::BindOnce(&LogOnUIThread, std::move(owned_event_name),
+                                  std::move(properties)));
+  }
+}
+
+}  // namespace browseros_metrics
