diff --git a/chrome/browser/chrome_content_browser_client.cc b/chrome/browser/chrome_content_browser_client.cc
index 90eec90d4ef45..da7beb4a4547e 100644
--- a/chrome/browser/chrome_content_browser_client.cc
+++ b/chrome/browser/chrome_content_browser_client.cc
@@ -598,6 +598,7 @@
 #endif
 
 #if BUILDFLAG(ENABLE_EXTENSIONS_CORE)
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/extensions/chrome_content_browser_client_extensions_part.h"
 #include "chrome/browser/extensions/chrome_extension_cookies.h"
 #include "extensions/browser/api/web_request/web_request_api.h"
@@ -1421,7 +1422,7 @@ void ChromeContentBrowserClient::RegisterLocalStatePrefs(
 void ChromeContentBrowserClient::RegisterProfilePrefs(
     user_prefs::PrefRegistrySyncable* registry) {
   registry->RegisterBooleanPref(prefs::kDisable3DAPIs, false);
-  registry->RegisterBooleanPref(prefs::kEnableHyperlinkAuditing, true);
+  registry->RegisterBooleanPref(prefs::kEnableHyperlinkAuditing, false);
   // Register user prefs for mapping SitePerProcess and IsolateOrigins in
   // user policy in addition to the same named ones in Local State (which are
   // used for mapping the command-line flags).
@@ -4884,6 +4885,43 @@ bool ChromeContentBrowserClient::
              prefs.root_scrollbar_theme_color;
 }
 
+// Handles chrome://browseros/* URLs by rewriting to extension URLs.
+// Forward handler: chrome://browseros/ai -> chrome-extension://[id]/options.html
+static bool HandleBrowserOSURL(GURL* url,
+                               content::BrowserContext* browser_context) {
+  if (!url->SchemeIs(content::kChromeUIScheme) ||
+      url->host() != browseros::kBrowserOSHost) {
+    return false;
+  }
+
+  std::string extension_url =
+      browseros::GetBrowserOSExtensionURL(url->path());
+  if (extension_url.empty()) {
+    return false;
+  }
+
+  *url = GURL(extension_url);
+  return true;
+}
+
+// Reverse handler: chrome-extension://[id]/options.html#ai -> chrome://browseros/ai
+// This ensures the virtual URL is shown in the address bar.
+static bool ReverseBrowserOSURL(GURL* url,
+                                content::BrowserContext* browser_context) {
+  if (!url->SchemeIs(extensions::kExtensionScheme)) {
+    return false;
+  }
+
+  std::string virtual_url = browseros::GetBrowserOSVirtualURL(
+      url->host(), url->path(), url->ref());
+  if (virtual_url.empty()) {
+    return false;
+  }
+
+  *url = GURL(virtual_url);
+  return true;
+}
+
 void ChromeContentBrowserClient::BrowserURLHandlerCreated(
     BrowserURLHandler* handler) {
   // The group policy NTP URL handler must be registered before the other NTP
@@ -4900,6 +4938,13 @@ void ChromeContentBrowserClient::BrowserURLHandlerCreated(
   handler->AddHandlerPair(&HandleChromeAboutAndChromeSyncRewrite,
                           BrowserURLHandler::null_handler());
 
+  // Handler to rewrite chrome://browseros/* to extension URLs.
+  handler->AddHandlerPair(&HandleBrowserOSURL, &ReverseBrowserOSURL);
+  // Reverse-only handler for when extension opens its URL directly
+  // (e.g., chrome.tabs.create({url: 'options.html#ai'}))
+  handler->AddHandlerPair(BrowserURLHandler::null_handler(),
+                          &ReverseBrowserOSURL);
+
 #if BUILDFLAG(IS_ANDROID)
   // Handler to rewrite chrome://newtab on Android.
   handler->AddHandlerPair(&chrome::android::HandleAndroidNativePageURL,
@@ -7693,6 +7738,15 @@ content::ContentBrowserClient::PrivateNetworkRequestPolicyOverride
 ChromeContentBrowserClient::ShouldOverridePrivateNetworkRequestPolicy(
     content::BrowserContext* browser_context,
     const url::Origin& origin) {
+#if BUILDFLAG(ENABLE_EXTENSIONS_CORE)
+  // Allow BrowserOS extensions to access private networks (e.g., localhost).
+  // This enables extension service workers to connect to local servers.
+  if (origin.scheme() == extensions::kExtensionScheme &&
+      browseros::IsBrowserOSExtension(origin.host())) {
+    return PrivateNetworkRequestPolicyOverride::kForceAllow;
+  }
+#endif
+
 #if BUILDFLAG(IS_ANDROID)
   if (base::android::device_info::is_automotive()) {
     return content::ContentBrowserClient::PrivateNetworkRequestPolicyOverride::
