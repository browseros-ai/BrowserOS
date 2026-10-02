diff --git a/chrome/browser/chrome_content_browser_client_navigation_throttles.cc b/chrome/browser/chrome_content_browser_client_navigation_throttles.cc
index cba0d32176eb2bfd1f36c9dcb336122195099d31..e53838e5cab50aca5adedbd88f6be46c2f3ddc52 100644
--- a/chrome/browser/chrome_content_browser_client_navigation_throttles.cc
+++ b/chrome/browser/chrome_content_browser_client_navigation_throttles.cc
@@ -95,6 +95,7 @@
 
 #else  // BUILDFLAG(IS_ANDROID)
 #include "chrome/browser/background/background_contents_navigation_throttle.h"
+#include "chrome/browser/browseros/onboarding/browseros_onboarding.h"
 #include "chrome/browser/devtools/devtools_navigation_throttle.h"
 #include "chrome/browser/page_info/web_view_side_panel_throttle.h"
 #include "chrome/browser/themes/theme_service_factory.h"
@@ -297,6 +298,12 @@ void CreateAndAddChromeThrottlesForNavigation(
     // NavigationThrottleRegistry::AddThrottle().
     page_load_metrics::MetricsNavigationThrottle::CreateAndAdd(registry);
 
+#if !BUILDFLAG(IS_ANDROID)
+    // Native onboarding owns the transition into the browsing window. Block
+    // the legacy renderer handoff until extension readiness has been handled.
+    BrowserOSOnboarding::MaybeCreateNavigationThrottle(registry);
+#endif
+
     // Appends the X-Geo header to the navigation request if needed.
     if (auto throttle =
             GeolocationNavigationThrottle::MaybeCreateThrottleFor(registry)) {
