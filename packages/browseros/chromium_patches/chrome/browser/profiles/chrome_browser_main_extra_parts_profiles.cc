diff --git a/chrome/browser/profiles/chrome_browser_main_extra_parts_profiles.cc b/chrome/browser/profiles/chrome_browser_main_extra_parts_profiles.cc
index fdb211c4c8ae2..ccd0f1b891a3e 100644
--- a/chrome/browser/profiles/chrome_browser_main_extra_parts_profiles.cc
+++ b/chrome/browser/profiles/chrome_browser_main_extra_parts_profiles.cc
@@ -52,6 +52,7 @@
 #include "chrome/browser/collaboration/messaging/messaging_backend_service_factory.h"
 #include "chrome/browser/commerce/shopping_service_factory.h"
 #include "chrome/browser/consent_auditor/consent_auditor_factory.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics_service_factory.h"
 #include "chrome/browser/content_index/content_index_provider_factory.h"
 #include "chrome/browser/content_settings/cookie_settings_factory.h"
 #include "chrome/browser/content_settings/host_content_settings_map_factory.h"
@@ -755,6 +756,7 @@ void ChromeBrowserMainExtraPartsProfiles::
 #endif
   BitmapFetcherServiceFactory::GetInstance();
   BluetoothChooserContextFactory::GetInstance();
+  browseros_metrics::BrowserOSMetricsServiceFactory::GetInstance();
 #if defined(TOOLKIT_VIEWS)
   BookmarkExpandedStateTrackerFactory::GetInstance();
   BookmarkMergedSurfaceServiceFactory::GetInstance();
