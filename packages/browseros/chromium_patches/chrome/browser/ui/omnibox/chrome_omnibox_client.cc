diff --git a/chrome/browser/ui/omnibox/chrome_omnibox_client.cc b/chrome/browser/ui/omnibox/chrome_omnibox_client.cc
index 3644a2c9a69cd163806e9058bf39a122d1f0812e..6e5b1d75147b211921112e3c8cf9c6f2adc43bd2 100644
--- a/chrome/browser/ui/omnibox/chrome_omnibox_client.cc
+++ b/chrome/browser/ui/omnibox/chrome_omnibox_client.cc
@@ -132,6 +132,7 @@
 #include "url/gurl.h"
 
 #if BUILDFLAG(ENABLE_EXTENSIONS)
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/ui/extensions/settings_api_bubble_helpers.h"
 
 #if BUILDFLAG(IS_WIN) || BUILDFLAG(IS_MAC)
@@ -474,15 +475,50 @@ gfx::Image ChromeOmniboxClient::GetSizedIcon(const gfx::Image& icon) const {
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
 
 bool ChromeOmniboxClient::IsContextualTasksPage() const {
