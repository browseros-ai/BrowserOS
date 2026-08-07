diff --git a/chrome/browser/ui/ui_features.cc b/chrome/browser/ui/ui_features.cc
index 6db423701529ad784468e3414b008f99c3a8d8bb..67d895e815c3456189963413aec2b420714744fa 100644
--- a/chrome/browser/ui/ui_features.cc
+++ b/chrome/browser/ui/ui_features.cc
@@ -67,7 +67,7 @@ BASE_FEATURE(kOfferPinToTaskbarInSettings, base::FEATURE_ENABLED_BY_DEFAULT);
 BASE_FEATURE(kOfferPinToTaskbarInfoBar, base::FEATURE_ENABLED_BY_DEFAULT);
 // Shows an infobar on PDFs offering to become the default PDF viewer if Chrome
 // isn't the default already.
-BASE_FEATURE(kPdfInfoBar, base::FEATURE_ENABLED_BY_DEFAULT);
+BASE_FEATURE(kPdfInfoBar, base::FEATURE_DISABLED_BY_DEFAULT);
 
 BASE_FEATURE(kSeparateDefaultAndPinPrompt, base::FEATURE_DISABLED_BY_DEFAULT);
 BASE_FEATURE_PARAM(int,
@@ -154,6 +154,10 @@ BASE_FEATURE_PARAM(int,
                    "max_distance_threshold",
                    20);
 
+BASE_FEATURE(kThirdPartyLlmPanel,
+             "ThirdPartyLlmPanel",
+             base::FEATURE_ENABLED_BY_DEFAULT);
+
 BASE_FEATURE(kTabDuplicateMetrics, base::FEATURE_ENABLED_BY_DEFAULT);
 
 // Enables tabs to be frozen when collapsed.
