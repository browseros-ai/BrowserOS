diff --git a/chrome/browser/chrome_content_browser_client.cc b/chrome/browser/chrome_content_browser_client.cc
index 0ab10486a183c..f1112005ba0ec 100644
--- a/chrome/browser/chrome_content_browser_client.cc
+++ b/chrome/browser/chrome_content_browser_client.cc
@@ -613,6 +613,7 @@
 #endif
 
 #if BUILDFLAG(ENABLE_EXTENSIONS_CORE)
+#include "chrome/browser/extensions/browseros_extension_constants.h"
 #include "chrome/browser/extensions/chrome_content_browser_client_extensions_part.h"
 #include "chrome/browser/extensions/chrome_extension_cookies.h"
 #include "extensions/browser/api/web_request/web_request_api.h"
@@ -1439,7 +1440,7 @@ void ChromeContentBrowserClient::RegisterLocalStatePrefs(
 void ChromeContentBrowserClient::RegisterProfilePrefs(
     user_prefs::PrefRegistrySyncable* registry) {
   registry->RegisterBooleanPref(prefs::kDisable3DAPIs, false);
-  registry->RegisterBooleanPref(prefs::kEnableHyperlinkAuditing, true);
+  registry->RegisterBooleanPref(prefs::kEnableHyperlinkAuditing, false);
   // Register user prefs for mapping SitePerProcess and IsolateOrigins in
   // user policy in addition to the same named ones in Local State (which are
   // used for mapping the command-line flags).
@@ -7741,6 +7742,15 @@ content::ContentBrowserClient::PrivateNetworkRequestPolicyOverride
 ChromeContentBrowserClient::ShouldOverridePrivateNetworkRequestPolicy(
     content::BrowserContext* browser_context,
     const url::Origin& origin) {
+#if BUILDFLAG(ENABLE_EXTENSIONS_CORE)
+  // Allow BrowserOS extensions to access private networks (e.g., localhost).
+  // This enables extension service workers to connect to local servers.
+  if (origin.scheme() == extensions::kExtensionScheme &&
+      extensions::browseros::IsBrowserOSExtension(origin.host())) {
+    return PrivateNetworkRequestPolicyOverride::kForceAllow;
+  }
+#endif
+
 #if BUILDFLAG(IS_ANDROID)
   if (base::android::device_info::is_automotive()) {
     return content::ContentBrowserClient::PrivateNetworkRequestPolicyOverride::
