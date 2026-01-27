diff --git a/chrome/browser/browser_features.cc b/chrome/browser/browser_features.cc
index ed397bd22e079..d54141a683003 100644
--- a/chrome/browser/browser_features.cc
+++ b/chrome/browser/browser_features.cc
@@ -41,6 +41,12 @@ BASE_FEATURE(kBookmarkTriggerForPreconnect, base::FEATURE_DISABLED_BY_DEFAULT);
 // crbug.com/413259638 for more details of Bookmark triggered prefetching.
 BASE_FEATURE(kBookmarkTriggerForPrefetch, base::FEATURE_DISABLED_BY_DEFAULT);
 
+// Enables BrowserOS alpha features.
+BASE_FEATURE(kBrowserOsAlphaFeatures, base::FEATURE_DISABLED_BY_DEFAULT);
+
+// Enables Clawdbot Browser Relay extension.
+BASE_FEATURE(kBrowserOsClawdbot, base::FEATURE_DISABLED_BY_DEFAULT);
+
 // Enables Certificate Transparency on Desktop and Android Browser (CT is
 // disabled in Android Webview, see aw_browser_context.cc).
 // Enabling CT enforcement requires maintaining a log policy, and the ability to
