diff --git a/chrome/browser/chrome_content_browser_client_navigation_throttles.cc b/chrome/browser/chrome_content_browser_client_navigation_throttles.cc
index 8de81338cce093a3fa4aebccaec37591c1b54976..0415111febcce81c7fbbda1f3bf809168da97e6f 100644
--- a/chrome/browser/chrome_content_browser_client_navigation_throttles.cc
+++ b/chrome/browser/chrome_content_browser_client_navigation_throttles.cc
@@ -89,6 +89,7 @@
 #endif  // BUILDFLAG(DFMIFY_DEV_UI)
 
 #else  // BUILDFLAG(IS_ANDROID)
+#include "chrome/browser/browseros/onboarding/browseros_onboarding.h"
 #include "chrome/browser/devtools/devtools_navigation_throttle.h"
 #include "chrome/browser/page_info/web_view_side_panel_throttle.h"
 #include "chrome/browser/themes/theme_service_factory.h"
@@ -290,6 +291,10 @@ void CreateAndAddChromeThrottlesForNavigation(
     // NavigationThrottleRegistry::AddThrottle().
     page_load_metrics::MetricsNavigationThrottle::CreateAndAdd(registry);
 
+#if !BUILDFLAG(IS_ANDROID)
+    BrowserOSOnboarding::MaybeCreateNavigationThrottle(registry);
+#endif
+
     // Appends the X-Geo header to the navigation request if needed.
     if (auto throttle =
             GeolocationNavigationThrottle::MaybeCreateThrottleFor(registry)) {
