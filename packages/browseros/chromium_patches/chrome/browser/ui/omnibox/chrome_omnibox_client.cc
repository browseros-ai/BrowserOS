diff --git a/chrome/browser/ui/omnibox/chrome_omnibox_client.cc b/chrome/browser/ui/omnibox/chrome_omnibox_client.cc
index 121eb4476bfcc..a1ee736be6b5f 100644
--- a/chrome/browser/ui/omnibox/chrome_omnibox_client.cc
+++ b/chrome/browser/ui/omnibox/chrome_omnibox_client.cc
@@ -109,6 +109,7 @@
 #include "url/gurl.h"
 
 #if BUILDFLAG(ENABLE_EXTENSIONS)
+#include "chrome/browser/extensions/browseros_extension_constants.h"
 #include "chrome/browser/ui/extensions/settings_api_bubble_helpers.h"
 #endif
 
@@ -298,7 +299,28 @@ std::u16string ChromeOmniboxClient::GetFormattedFullURL() const {
 }
 
 std::u16string ChromeOmniboxClient::GetURLForDisplay() const {
-  return location_bar_->GetLocationBarModel()->GetURLForDisplay();
+  std::u16string display_url =
+      location_bar_->GetLocationBarModel()->GetURLForDisplay();
+
+#if BUILDFLAG(ENABLE_EXTENSIONS)
+  // Transform BlockBrowser extension URLs to display custom names
+  GURL url = location_bar_->GetLocationBarModel()->GetURL();
+  if (url.SchemeIs(extensions::kExtensionScheme)) {
+    std::string extension_id = url.host();
+    if (auto display_name =
+            extensions::browseros::GetExtensionDisplayName(extension_id)) {
+      std::u16string result = base::UTF8ToUTF16(display_name.value());
+      // Append path if present (e.g., "BlockBrowser-Agent/popup.html")
+      if (url.has_path() && url.path() != "/") {
+        result += u"/";
+        result += base::UTF8ToUTF16(url.path().substr(1));  // Skip leading '/'
+      }
+      return result;
+    }
+  }
+#endif
+
+  return display_url;
 }
 
 GURL ChromeOmniboxClient::GetNavigationEntryURL() const {
