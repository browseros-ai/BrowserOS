diff --git a/chrome/browser/browser_features.h b/chrome/browser/browser_features.h
index f6ba98b4b5902f4d3e297a31a6f1b8ea4ecc46a6..5ad135a091f55aeb096944959637b6c0a92c37e0 100644
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
