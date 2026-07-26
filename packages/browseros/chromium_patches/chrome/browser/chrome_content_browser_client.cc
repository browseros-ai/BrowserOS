diff --git a/chrome/browser/chrome_content_browser_client.cc b/chrome/browser/chrome_content_browser_client.cc
index f823433796c5c..1bc6610d0626e 100644
--- a/chrome/browser/chrome_content_browser_client.cc
+++ b/chrome/browser/chrome_content_browser_client.cc
@@ -623,6 +623,7 @@
 #endif
 
 #if BUILDFLAG(ENABLE_EXTENSIONS_CORE)
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/extensions/chrome_content_browser_client_extensions_part.h"
 #include "chrome/browser/extensions/chrome_extension_cookies.h"
 #include "extensions/browser/api/web_request/web_request_api.h"
@@ -1541,7 +1542,7 @@ void ChromeContentBrowserClient::RegisterLocalStatePrefs(
 void ChromeContentBrowserClient::RegisterProfilePrefs(
     user_prefs::PrefRegistrySyncable* registry) {
   registry->RegisterBooleanPref(prefs::kDisable3DAPIs, false);
-  registry->RegisterBooleanPref(prefs::kEnableHyperlinkAuditing, true);
+  registry->RegisterBooleanPref(prefs::kEnableHyperlinkAuditing, false);
   // Register user prefs for mapping SitePerProcess and IsolateOrigins in
   // user policy in addition to the same named ones in Local State (which are
   // used for mapping the command-line flags).
@@ -4479,10 +4480,10 @@ bool ChromeContentBrowserClient::CanCreateWindow(
         web_contents->GetResponsibleWebContents();
     bool is_from_embedded_page = web_contents != responsible_web_contents;
     if (contextual_tasks_ui_service &&
-        contextual_tasks_ui_service->HandleNavigation(
-            std::move(url_params), responsible_web_contents,
-            is_from_embedded_page,
-            /*is_to_new_tab=*/true)) {
+        contextual_tasks_ui_service->HandleNavigation(std::move(url_params),
+                                                      responsible_web_contents,
+                                                      is_from_embedded_page,
+                                                      /*is_to_new_tab=*/true)) {
       return false;
     }
   }
@@ -5052,6 +5053,44 @@ bool ChromeContentBrowserClient::
              prefs.root_scrollbar_theme_color;
 }
 
+// Handles chrome://browseros/* URLs by rewriting to extension URLs.
+// Forward handler: chrome://browseros/ai ->
+// chrome-extension://[id]/options.html
+static bool HandleBrowserOSURL(GURL* url,
+                               content::BrowserContext* browser_context) {
+  if (!url->SchemeIs(content::kChromeUIScheme) ||
+      url->host() != browseros::kBrowserOSHost) {
+    return false;
+  }
+
+  std::string extension_url = browseros::GetBrowserOSExtensionURL(url->path());
+  if (extension_url.empty()) {
+    return false;
+  }
+
+  *url = GURL(extension_url);
+  return true;
+}
+
+// Reverse handler: chrome-extension://[id]/options.html#ai ->
+// chrome://browseros/ai This ensures the virtual URL is shown in the address
+// bar.
+static bool ReverseBrowserOSURL(GURL* url,
+                                content::BrowserContext* browser_context) {
+  if (!url->SchemeIs(extensions::kExtensionScheme)) {
+    return false;
+  }
+
+  std::string virtual_url =
+      browseros::GetBrowserOSVirtualURL(url->host(), url->path(), url->ref());
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
@@ -5068,6 +5107,13 @@ void ChromeContentBrowserClient::BrowserURLHandlerCreated(
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
@@ -7901,6 +7947,15 @@ content::ContentBrowserClient::LocalNetworkAccessRequestPolicyOverride
 ChromeContentBrowserClient::ShouldOverrideLocalNetworkAccessRequestPolicy(
     content::BrowserContext* browser_context,
     const url::Origin& origin) {
+#if BUILDFLAG(ENABLE_EXTENSIONS_CORE)
+  // Allow BrowserOS extensions to access private networks (e.g., localhost).
+  // This enables extension service workers to connect to local servers.
+  if (origin.scheme() == extensions::kExtensionScheme &&
+      browseros::IsActiveBrowserOSExtension(origin.host())) {
+    return LocalNetworkAccessRequestPolicyOverride::kForceAllow;
+  }
+#endif
+
 #if BUILDFLAG(IS_ANDROID)
   if (base::android::device_info::is_automotive()) {
     return content::ContentBrowserClient::
