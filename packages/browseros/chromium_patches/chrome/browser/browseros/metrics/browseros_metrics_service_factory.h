diff --git a/chrome/browser/browseros/metrics/browseros_metrics_service_factory.h b/chrome/browser/browseros/metrics/browseros_metrics_service_factory.h
new file mode 100644
index 0000000000000..2caddc7598a43
--- /dev/null
+++ b/chrome/browser/browseros/metrics/browseros_metrics_service_factory.h
@@ -0,0 +1,48 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_SERVICE_FACTORY_H_
+#define CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_SERVICE_FACTORY_H_
+
+#include "base/no_destructor.h"
+#include "components/keyed_service/content/browser_context_keyed_service_factory.h"
+
+namespace content {
+class BrowserContext;
+}  // namespace content
+
+namespace browseros_metrics {
+
+class BrowserOSMetricsService;
+
+// Factory for creating BrowserOSMetricsService instances per profile.
+class BrowserOSMetricsServiceFactory
+    : public BrowserContextKeyedServiceFactory {
+ public:
+  BrowserOSMetricsServiceFactory(const BrowserOSMetricsServiceFactory&) =
+      delete;
+  BrowserOSMetricsServiceFactory& operator=(
+      const BrowserOSMetricsServiceFactory&) = delete;
+
+  // Returns the BrowserOSMetricsService for |context|, creating one if needed.
+  static BrowserOSMetricsService* GetForBrowserContext(
+      content::BrowserContext* context);
+
+  // Returns the singleton factory instance.
+  static BrowserOSMetricsServiceFactory* GetInstance();
+
+ private:
+  friend base::NoDestructor<BrowserOSMetricsServiceFactory>;
+
+  BrowserOSMetricsServiceFactory();
+  ~BrowserOSMetricsServiceFactory() override;
+
+  // BrowserContextKeyedServiceFactory:
+  std::unique_ptr<KeyedService> BuildServiceInstanceForBrowserContext(
+      content::BrowserContext* context) const override;
+};
+
+}  // namespace browseros_metrics
+
+#endif  // CHROME_BROWSER_BROWSEROS_METRICS_BROWSEROS_METRICS_SERVICE_FACTORY_H_
