diff --git a/chrome/common/webui_url_constants.cc b/chrome/common/webui_url_constants.cc
index 17100eb74d54ccb2b0a69ead43667b901cca9c76..0418f0687e017ff01c7c57108a138c429e24fb7b 100644
--- a/chrome/common/webui_url_constants.cc
+++ b/chrome/common/webui_url_constants.cc
@@ -128,6 +128,7 @@ base::span<const base::cstring_view> ChromeURLHosts() {
 #endif
       kChromeUIAutofillInternalsHost,
       kChromeUIBluetoothInternalsHost,
+      kChromeUIBrowserOSOnboardingHost,
       kChromeUIChromeURLsHost,
       kChromeUIComponentsHost,
       commerce::kChromeUICommerceInternalsHost,
