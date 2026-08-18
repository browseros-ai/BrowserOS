diff --git a/chrome/browser/browser_features.h b/chrome/browser/browser_features.h
index 5a009faf7bdde1fd7dc0cc52dce66610322028a2..257aefc5cf206e79c885ef195ccd0ed2d90ac830 100644
--- a/chrome/browser/browser_features.h
+++ b/chrome/browser/browser_features.h
@@ -34,6 +34,8 @@ BASE_DECLARE_FEATURE(kAllowUnmutedAutoplayForTWA);
 #endif  // BUILDFLAG(IS_ANDROID)
 BASE_DECLARE_FEATURE(kAutocompleteActionPredictorConfidenceCutoff);
 BASE_DECLARE_FEATURE(kBookmarkTriggerForPrerender2KillSwitch);
+BASE_DECLARE_FEATURE(kBrowserOsAlphaFeatures);
+BASE_DECLARE_FEATURE(kBrowserOsKeyboardShortcuts);
 BASE_DECLARE_FEATURE(kBookmarkTriggerForPreconnect);
 BASE_DECLARE_FEATURE(kBookmarkTriggerForPrefetch);
 BASE_DECLARE_FEATURE(kCertificateTransparencyAskBeforeEnabling);
