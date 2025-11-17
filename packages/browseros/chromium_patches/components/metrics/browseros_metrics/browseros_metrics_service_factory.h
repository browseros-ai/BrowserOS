diff --git a/components/metrics/blockbrowser_metrics/blockbrowser_metrics_service_factory.h b/components/metrics/blockbrowser_metrics/blockbrowser_metrics_service_factory.h
new file mode 100644
index 0000000000000..014eb17aba442
--- /dev/null
+++ b/components/metrics/blockbrowser_metrics/blockbrowser_metrics_service_factory.h
@@ -0,0 +1,48 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef COMPONENTS_METRICS_BROWSEROS_METRICS_BROWSEROS_METRICS_SERVICE_FACTORY_H_
+#define COMPONENTS_METRICS_BROWSEROS_METRICS_BROWSEROS_METRICS_SERVICE_FACTORY_H_
+
+#include "base/no_destructor.h"
+#include "components/keyed_service/content/browser_context_keyed_service_factory.h"
+
+namespace content {
+class BrowserContext;
+}  // namespace content
+
+namespace blockbrowser_metrics {
+
+class BlockBrowserMetricsService;
+
+// Factory for creating BlockBrowserMetricsService instances per profile.
+class BlockBrowserMetricsServiceFactory
+    : public BrowserContextKeyedServiceFactory {
+ public:
+  BlockBrowserMetricsServiceFactory(const BlockBrowserMetricsServiceFactory&) =
+      delete;
+  BlockBrowserMetricsServiceFactory& operator=(
+      const BlockBrowserMetricsServiceFactory&) = delete;
+
+  // Returns the BlockBrowserMetricsService for |context|, creating one if needed.
+  static BlockBrowserMetricsService* GetForBrowserContext(
+      content::BrowserContext* context);
+
+  // Returns the singleton factory instance.
+  static BlockBrowserMetricsServiceFactory* GetInstance();
+
+ private:
+  friend base::NoDestructor<BlockBrowserMetricsServiceFactory>;
+
+  BlockBrowserMetricsServiceFactory();
+  ~BlockBrowserMetricsServiceFactory() override;
+
+  // BrowserContextKeyedServiceFactory:
+  std::unique_ptr<KeyedService> BuildServiceInstanceForBrowserContext(
+      content::BrowserContext* context) const override;
+};
+
+}  // namespace blockbrowser_metrics
+
+#endif  // COMPONENTS_METRICS_BROWSEROS_METRICS_BROWSEROS_METRICS_SERVICE_FACTORY_H_
\ No newline at end of file
