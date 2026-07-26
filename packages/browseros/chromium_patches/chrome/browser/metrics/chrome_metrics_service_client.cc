diff --git a/chrome/browser/metrics/chrome_metrics_service_client.cc b/chrome/browser/metrics/chrome_metrics_service_client.cc
index c1c24f499ae2a..7132b8186a7d8 100644
--- a/chrome/browser/metrics/chrome_metrics_service_client.cc
+++ b/chrome/browser/metrics/chrome_metrics_service_client.cc
@@ -35,6 +35,7 @@
 #include "base/threading/platform_thread.h"
 #include "base/time/time.h"
 #include "build/build_config.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics.h"
 #include "chrome/browser/browser_process.h"
 #include "chrome/browser/glic/glic_metrics_provider.h"
 #include "chrome/browser/google/google_brand.h"
@@ -1116,6 +1117,7 @@ void ChromeMetricsServiceClient::RegisterUKMProviders() {
 }
 
 void ChromeMetricsServiceClient::NotifyApplicationNotIdle() {
+  browseros_metrics::BrowserOSMetrics::Log("alive", 0.01);
   metrics_service_->OnApplicationNotIdle();
 }
 
