diff --git a/chrome/browser/browser_features.cc b/chrome/browser/browser_features.cc
index fda2bec4c1002880bee0c464ee0a025082cc3cd6..8ee5d8e8c564cda9a297df6bfa227978aba2dd01 100644
--- a/chrome/browser/browser_features.cc
+++ b/chrome/browser/browser_features.cc
@@ -33,6 +33,13 @@ BASE_FEATURE(kBookmarkTriggerForPreconnect, base::FEATURE_ENABLED_BY_DEFAULT);
 // crbug.com/413259638 for more details of Bookmark triggered prefetching.
 BASE_FEATURE(kBookmarkTriggerForPrefetch, base::FEATURE_DISABLED_BY_DEFAULT);
 
+// Enables BrowserOS alpha features.
+BASE_FEATURE(kBrowserOsAlphaFeatures, base::FEATURE_DISABLED_BY_DEFAULT);
+
+// Enables BrowserOS keyboard shortcuts (Option+K, Option+L, Option+A, etc.).
+// Disable this on keyboards where Option+letter produces special characters.
+BASE_FEATURE(kBrowserOsKeyboardShortcuts, base::FEATURE_ENABLED_BY_DEFAULT);
+
 // Enables Certificate Transparency on Desktop and Android Browser (CT is
 // disabled in Android Webview, see aw_browser_context.cc).
 // Enabling CT enforcement requires maintaining a log policy, and the ability to
