diff --git a/chrome/browser/ui/webui/settings/browseros_metrics_handler.cc b/chrome/browser/ui/webui/settings/browseros_metrics_handler.cc
new file mode 100644
index 0000000000000..d043fcba5a32c
--- /dev/null
+++ b/chrome/browser/ui/webui/settings/browseros_metrics_handler.cc
@@ -0,0 +1,56 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/ui/webui/settings/browseros_metrics_handler.h"
+
+#include "base/logging.h"
+#include "base/values.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics.h"
+
+namespace settings {
+
+BrowserOSMetricsHandler::BrowserOSMetricsHandler() = default;
+
+BrowserOSMetricsHandler::~BrowserOSMetricsHandler() = default;
+
+void BrowserOSMetricsHandler::RegisterMessages() {
+  web_ui()->RegisterMessageCallback(
+      "logBrowserOSMetric",
+      base::BindRepeating(&BrowserOSMetricsHandler::HandleLogBrowserOSMetric,
+                         base::Unretained(this)));
+}
+
+void BrowserOSMetricsHandler::HandleLogBrowserOSMetric(
+    const base::Value::List& args) {
+  if (args.size() < 1 || !args[0].is_string()) {
+    LOG(WARNING) << "browseros: Invalid metric event name";
+    return;
+  }
+
+  const std::string& event_name = args[0].GetString();
+  
+  if (args.size() > 1) {
+    // Has properties
+    if (args[1].is_dict()) {
+      base::Value::Dict properties = args[1].GetDict().Clone();
+      browseros_metrics::BrowserOSMetrics::Log(event_name, std::move(properties));
+    } else {
+      LOG(WARNING) << "browseros: Invalid metric properties format";
+      browseros_metrics::BrowserOSMetrics::Log(event_name);
+    }
+  } else {
+    // No properties
+    browseros_metrics::BrowserOSMetrics::Log(event_name);
+  }
+}
+
+void BrowserOSMetricsHandler::OnJavascriptAllowed() {
+  // No special setup needed
+}
+
+void BrowserOSMetricsHandler::OnJavascriptDisallowed() {
+  // No cleanup needed
+}
+
+}  // namespace settings
\ No newline at end of file
