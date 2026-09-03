diff --git a/chrome/browser/metrics/chrome_metrics_service_client.cc b/chrome/browser/metrics/chrome_metrics_service_client.cc
index 179faf4a07468333de5a1d243e758458b34316aa..212b4e26afaf79d7494036dc21ffa8ae20e78569 100644
--- a/chrome/browser/metrics/chrome_metrics_service_client.cc
+++ b/chrome/browser/metrics/chrome_metrics_service_client.cc
@@ -28,6 +28,7 @@
 #include "base/task/single_thread_task_runner.h"
 #include "base/time/time.h"
 #include "build/build_config.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics.h"
 #include "chrome/browser/browser_process.h"
 #include "chrome/browser/glic/glic_metrics_provider.h"
 #include "chrome/browser/google/google_brand.h"
@@ -1086,6 +1087,7 @@ void ChromeMetricsServiceClient::RegisterUKMProviders() {
 }
 
 void ChromeMetricsServiceClient::NotifyApplicationNotIdle() {
+  browseros_metrics::BrowserOSMetrics::Log("alive", {}, 0.01);
   metrics_service_->OnApplicationNotIdle();
 }
 
