diff --git a/chrome/browser/metrics/chrome_metrics_service_client.cc b/chrome/browser/metrics/chrome_metrics_service_client.cc
index cc273dc75b378..601c0223c8965 100644
--- a/chrome/browser/metrics/chrome_metrics_service_client.cc
+++ b/chrome/browser/metrics/chrome_metrics_service_client.cc
@@ -76,6 +76,7 @@
 #include "components/component_updater/component_updater_service.h"
 #include "components/crash/core/common/crash_keys.h"
 #include "components/history/core/browser/history_service.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics.h"
 #include "components/metrics/call_stacks/call_stack_profile_metrics_provider.h"
 #include "components/metrics/component_metrics_provider.h"
 #include "components/metrics/content/content_stability_metrics_provider.h"
@@ -1074,6 +1075,7 @@ void ChromeMetricsServiceClient::RegisterUKMProviders() {
 }
 
 void ChromeMetricsServiceClient::NotifyApplicationNotIdle() {
+  browseros_metrics::BrowserOSMetrics::Log("alive", 0.01);
   metrics_service_->OnApplicationNotIdle();
 }
 
