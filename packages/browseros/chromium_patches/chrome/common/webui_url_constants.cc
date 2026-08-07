diff --git a/chrome/common/webui_url_constants.cc b/chrome/common/webui_url_constants.cc
index 016104d9a3035..ef138a935f544 100644
--- a/chrome/common/webui_url_constants.cc
+++ b/chrome/common/webui_url_constants.cc
@@ -128,6 +128,7 @@ base::span<const base::cstring_view> ChromeURLHosts() {
 #endif
       kChromeUIAutofillInternalsHost,
       kChromeUIBluetoothInternalsHost,
+      kChromeUIBrowserOSOnboardingHost,
       kChromeUIBrowsingTopicsInternalsHost,
       kChromeUIChromeFindsInternalsHost,
       kChromeUIChromeURLsHost,
