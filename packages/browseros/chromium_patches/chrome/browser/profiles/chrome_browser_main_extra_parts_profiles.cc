diff --git a/chrome/browser/profiles/chrome_browser_main_extra_parts_profiles.cc b/chrome/browser/profiles/chrome_browser_main_extra_parts_profiles.cc
index 1569acc6c6d9a..46fbc3bad903d 100644
--- a/chrome/browser/profiles/chrome_browser_main_extra_parts_profiles.cc
+++ b/chrome/browser/profiles/chrome_browser_main_extra_parts_profiles.cc
@@ -62,6 +62,7 @@
 #include "chrome/browser/collaboration/messaging/messaging_backend_service_factory.h"
 #include "chrome/browser/commerce/shopping_service_factory.h"
 #include "chrome/browser/consent_auditor/consent_auditor_factory.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics_service_factory.h"
 #include "chrome/browser/content_index/content_index_provider_factory.h"
 #include "chrome/browser/content_settings/cookie_settings_factory.h"
 #include "chrome/browser/content_settings/host_content_settings_map_factory.h"
@@ -775,6 +776,7 @@ void ChromeBrowserMainExtraPartsProfiles::
 #endif
   BitmapFetcherServiceFactory::GetInstance();
   BluetoothChooserContextFactory::GetInstance();
+  browseros_metrics::BrowserOSMetricsServiceFactory::GetInstance();
 #if defined(TOOLKIT_VIEWS)
   BookmarkExpandedStateTrackerFactory::GetInstance();
   BookmarkMergedSurfaceServiceFactory::GetInstance();
