diff --git a/chrome/browser/metrics/chrome_metrics_service_client.cc b/chrome/browser/metrics/chrome_metrics_service_client.cc
index ea4e1a621b201..21a7a3aef2872 100644
--- a/chrome/browser/metrics/chrome_metrics_service_client.cc
+++ b/chrome/browser/metrics/chrome_metrics_service_client.cc
@@ -75,6 +75,7 @@
 #include "components/component_updater/component_updater_service.h"
 #include "components/crash/core/common/crash_keys.h"
 #include "components/history/core/browser/history_service.h"
+#include "components/metrics/blockbrowser_metrics/blockbrowser_metrics.h"
 #include "components/metrics/call_stacks/call_stack_profile_metrics_provider.h"
 #include "components/metrics/component_metrics_provider.h"
 #include "components/metrics/content/content_stability_metrics_provider.h"
@@ -1041,6 +1042,7 @@ void ChromeMetricsServiceClient::RegisterUKMProviders() {
 }
 
 void ChromeMetricsServiceClient::NotifyApplicationNotIdle() {
+  blockbrowser_metrics::BlockBrowserMetrics::Log("alive", 0.01);
   metrics_service_->OnApplicationNotIdle();
 }
 
