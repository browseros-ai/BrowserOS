diff --git a/chrome/browser/profiles/chrome_browser_main_extra_parts_profiles.cc b/chrome/browser/profiles/chrome_browser_main_extra_parts_profiles.cc
index c6e46fb1d8030..f06e6e0e07cfa 100644
--- a/chrome/browser/profiles/chrome_browser_main_extra_parts_profiles.cc
+++ b/chrome/browser/profiles/chrome_browser_main_extra_parts_profiles.cc
@@ -49,6 +49,7 @@
 #include "chrome/browser/collaboration/messaging/messaging_backend_service_factory.h"
 #include "chrome/browser/commerce/shopping_service_factory.h"
 #include "chrome/browser/consent_auditor/consent_auditor_factory.h"
+#include "components/metrics/blockbrowser_metrics/blockbrowser_metrics_service_factory.h"
 #include "chrome/browser/content_index/content_index_provider_factory.h"
 #include "chrome/browser/content_settings/cookie_settings_factory.h"
 #include "chrome/browser/content_settings/host_content_settings_map_factory.h"
@@ -722,6 +723,7 @@ void ChromeBrowserMainExtraPartsProfiles::
 #endif
   BitmapFetcherServiceFactory::GetInstance();
   BluetoothChooserContextFactory::GetInstance();
+  blockbrowser_metrics::BlockBrowserMetricsServiceFactory::GetInstance();
 #if defined(TOOLKIT_VIEWS)
   BookmarkExpandedStateTrackerFactory::GetInstance();
   BookmarkMergedSurfaceServiceFactory::GetInstance();
