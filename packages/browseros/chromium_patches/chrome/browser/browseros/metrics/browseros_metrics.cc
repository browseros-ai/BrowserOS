diff --git a/chrome/browser/browseros/metrics/browseros_metrics.cc b/chrome/browser/browseros/metrics/browseros_metrics.cc
new file mode 100644
index 0000000000000..cf1a9a0c1790b
--- /dev/null
+++ b/chrome/browser/browseros/metrics/browseros_metrics.cc
@@ -0,0 +1,100 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/metrics/browseros_metrics.h"
+
+#include "base/logging.h"
+#include "base/rand_util.h"
+#include "base/task/thread_pool.h"
+#include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics_service.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics_service_factory.h"
+#include "chrome/browser/profiles/profile_manager.h"
+#include "content/public/browser/browser_task_traits.h"
+#include "content/public/browser/browser_thread.h"
+
+namespace browseros_metrics {
+
+namespace {
+
+// Helper to get the metrics service
+BrowserOSMetricsService* GetMetricsService() {
+  // Must be called on UI thread
+  if (!content::BrowserThread::CurrentlyOn(content::BrowserThread::UI)) {
+    return nullptr;
+  }
+
+  // Get the profile manager
+  ProfileManager* profile_manager = g_browser_process->profile_manager();
+  if (!profile_manager) {
+    return nullptr;
+  }
+
+  // Get the last used profile (or the default one)
+  Profile* profile = profile_manager->GetLastUsedProfile();
+  if (!profile || profile->IsOffTheRecord()) {
+    return nullptr;
+  }
+
+  // Get the metrics service
+  return BrowserOSMetricsServiceFactory::GetForBrowserContext(profile);
+}
+
+void LogOnUIThread(const std::string& event_name, base::Value::Dict properties) {
+  auto* service = GetMetricsService();
+  if (service) {
+    service->CaptureEvent(event_name, std::move(properties));
+  } else {
+    VLOG(1) << "browseros: Metrics service not available for event: " << event_name;
+  }
+}
+
+}  // namespace
+
+// static
+void BrowserOSMetrics::Log(const std::string& event_name, double sample_rate) {
+  Log(event_name, base::Value::Dict(), sample_rate);
+}
+
+// static
+void BrowserOSMetrics::Log(const std::string& event_name,
+                           std::initializer_list<std::pair<std::string, base::Value>> properties,
+                           double sample_rate) {
+  base::Value::Dict dict;
+  for (const auto& [key, value] : properties) {
+    dict.Set(key, value.Clone());
+  }
+  Log(event_name, std::move(dict), sample_rate);
+}
+
+// static
+void BrowserOSMetrics::Log(const std::string& event_name, base::Value::Dict properties,
+                           double sample_rate) {
+  if (sample_rate <= 0.0 || sample_rate > 1.0) {
+    return;
+  }
+
+  if (sample_rate < 1.0) {
+    double random_value = base::RandDouble();
+    if (random_value > sample_rate) {
+      return;
+    }
+  }
+
+  if (sample_rate < 1.0) {
+    properties.Set("sample_rate", sample_rate);
+  }
+
+  // If we're already on the UI thread, log directly
+  if (content::BrowserThread::CurrentlyOn(content::BrowserThread::UI)) {
+    LogOnUIThread(event_name, std::move(properties));
+  } else {
+    // Post to UI thread
+    content::GetUIThreadTaskRunner({})->PostTask(
+        FROM_HERE,
+        base::BindOnce(&LogOnUIThread, event_name, std::move(properties)));
+  }
+}
+
+}  // namespace browseros_metrics
