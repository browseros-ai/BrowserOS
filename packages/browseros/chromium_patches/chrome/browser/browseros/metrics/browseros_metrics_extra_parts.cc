diff --git a/chrome/browser/browseros/metrics/browseros_metrics_extra_parts.cc b/chrome/browser/browseros/metrics/browseros_metrics_extra_parts.cc
new file mode 100644
index 0000000000000000000000000000000000000000..c2c5dd301d6f957a6581ad0d19b233ed7edaa2c9
--- /dev/null
+++ b/chrome/browser/browseros/metrics/browseros_metrics_extra_parts.cc
@@ -0,0 +1,63 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/metrics/browseros_metrics_extra_parts.h"
+
+#include <memory>
+
+#include "base/check.h"
+#include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_product.h"
+#include "chrome/browser/browseros/core/browseros_product_state.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics_service.h"
+#include "chrome/browser/chrome_browser_main.h"
+#include "chrome/browser/chrome_browser_main_extra_parts.h"
+#include "services/network/public/cpp/shared_url_loader_factory.h"
+
+namespace browseros_metrics {
+namespace {
+
+// Bridges BrowserOSMetricsService to the browser process lifecycle. The extra
+// part is registered before profile creation and is its sole owner, keeping
+// analytics independent of whichever profile happens to be active.
+class BrowserOSMetricsExtraParts final : public ChromeBrowserMainExtraParts {
+ public:
+  BrowserOSMetricsExtraParts() = default;
+  ~BrowserOSMetricsExtraParts() override { ShutdownMetrics(); }
+
+  void PreProfileInit() override {
+    CHECK(!metrics_service_);
+    const browseros::Product product = browseros::GetProduct();
+    metrics_service_ = std::make_unique<BrowserOSMetricsService>(
+        product, browseros::GetProductStateDirectory(product),
+        g_browser_process->shared_url_loader_factory());
+    SetBrowserOSMetricsServiceForProcess(metrics_service_.get());
+  }
+
+  void PostMainMessageLoopRun() override { ShutdownMetrics(); }
+
+ private:
+  void ShutdownMetrics() {
+    if (!metrics_service_) {
+      return;
+    }
+
+    // Stop global lookup before cancellation so shutdown-time callers cannot
+    // enqueue work into a service whose network context is being dismantled.
+    SetBrowserOSMetricsServiceForProcess(nullptr);
+    metrics_service_->Shutdown();
+    metrics_service_.reset();
+  }
+
+  std::unique_ptr<BrowserOSMetricsService> metrics_service_;
+};
+
+}  // namespace
+
+void AddBrowserOSMetricsExtraParts(ChromeBrowserMainParts* main_parts) {
+  CHECK(main_parts);
+  main_parts->AddParts(std::make_unique<BrowserOSMetricsExtraParts>());
+}
+
+}  // namespace browseros_metrics
