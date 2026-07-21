diff --git a/chrome/common/webui_url_constants.cc b/chrome/common/webui_url_constants.cc
index 4c12bc642955f1c2ff6f618e70a0ce60374afb1f..45ea55c350ce831db00661310572436190db588c 100644
--- a/chrome/common/webui_url_constants.cc
+++ b/chrome/common/webui_url_constants.cc
@@ -129,6 +129,7 @@ base::span<const base::cstring_view> ChromeURLHosts() {
 #endif
       kChromeUIAutofillInternalsHost,
       kChromeUIBluetoothInternalsHost,
+      kChromeUIBrowserOSOnboardingHost,
       kChromeUIBrowsingTopicsInternalsHost,
       kChromeUIChromeFindsInternalsHost,
       kChromeUIChromeURLsHost,
