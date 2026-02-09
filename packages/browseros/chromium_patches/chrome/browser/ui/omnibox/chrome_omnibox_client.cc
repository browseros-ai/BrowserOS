diff --git a/chrome/browser/ui/omnibox/chrome_omnibox_client.cc b/chrome/browser/ui/omnibox/chrome_omnibox_client.cc
index 37cb48b9b7c9f..90d2d430f41a5 100644
--- a/chrome/browser/ui/omnibox/chrome_omnibox_client.cc
+++ b/chrome/browser/ui/omnibox/chrome_omnibox_client.cc
@@ -123,6 +123,7 @@
 #include "url/gurl.h"
 
 #if BUILDFLAG(ENABLE_EXTENSIONS)
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/ui/extensions/settings_api_bubble_helpers.h"
 #endif
 
@@ -323,15 +324,50 @@ gfx::Image ChromeOmniboxClient::GetSizedIcon(const gfx::Image& icon) const {
 }
 
 std::u16string ChromeOmniboxClient::GetFormattedFullURL() const {
+#if BUILDFLAG(ENABLE_EXTENSIONS)
+  // Transform BrowserOS extension URLs to chrome://browseros/* virtual URLs
+  GURL url = location_bar_->GetLocationBarModel()->GetURL();
+  if (url.SchemeIs(extensions::kExtensionScheme)) {
+    std::string virtual_url = browseros::GetBrowserOSVirtualURL(
+        url.host(), url.path(), url.ref());
+    if (!virtual_url.empty()) {
+      return base::UTF8ToUTF16(virtual_url);
+    }
+  }
+#endif
   return location_bar_->GetLocationBarModel()->GetFormattedFullURL();
 }
 
 std::u16string ChromeOmniboxClient::GetURLForDisplay() const {
+#if BUILDFLAG(ENABLE_EXTENSIONS)
+  // Transform BrowserOS extension URLs to chrome://browseros/* virtual URLs
+  GURL url = location_bar_->GetLocationBarModel()->GetURL();
+  if (url.SchemeIs(extensions::kExtensionScheme)) {
+    std::string virtual_url = browseros::GetBrowserOSVirtualURL(
+        url.host(), url.path(), url.ref());
+    if (!virtual_url.empty()) {
+      return base::UTF8ToUTF16(virtual_url);
+    }
+  }
+#endif
   return location_bar_->GetLocationBarModel()->GetURLForDisplay();
 }
 
 GURL ChromeOmniboxClient::GetNavigationEntryURL() const {
+#if BUILDFLAG(ENABLE_EXTENSIONS)
+  // Transform BrowserOS extension URLs to chrome://browseros/* virtual URLs
+  GURL url = location_bar_->GetLocationBarModel()->GetURL();
+  if (url.SchemeIs(extensions::kExtensionScheme)) {
+    std::string virtual_url = browseros::GetBrowserOSVirtualURL(
+        url.host(), url.path(), url.ref());
+    if (!virtual_url.empty()) {
+      return GURL(virtual_url);
+    }
+  }
+  return url;
+#else
   return location_bar_->GetLocationBarModel()->GetURL();
+#endif
 }
 
 metrics::OmniboxEventProto::PageClassification
