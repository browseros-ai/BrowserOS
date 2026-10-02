diff --git a/chrome/browser/ui/ui_features.cc b/chrome/browser/ui/ui_features.cc
index 24f6808a21b00c50a101a1a4c0100ff80561e227..2e97ba8345c24bfd0f59672ff5d726bf007dae5d 100644
--- a/chrome/browser/ui/ui_features.cc
+++ b/chrome/browser/ui/ui_features.cc
@@ -147,7 +147,7 @@ BASE_FEATURE(kExtensionsPinnedByDefault, base::FEATURE_DISABLED_BY_DEFAULT);
 #if BUILDFLAG(IS_WIN) || BUILDFLAG(IS_MAC)
 // Shows an infobar on PDFs offering to become the default PDF viewer if Chrome
 // isn't the default already.
-BASE_FEATURE(kPdfInfoBar, base::FEATURE_ENABLED_BY_DEFAULT);
+BASE_FEATURE(kPdfInfoBar, base::FEATURE_DISABLED_BY_DEFAULT);
 
 BASE_FEATURE(kSeparateDefaultAndPinPrompt,
 #if BUILDFLAG(IS_WIN)
@@ -244,6 +244,10 @@ BASE_FEATURE_PARAM(int,
                    "max_distance_threshold",
                    20);
 
+BASE_FEATURE(kThirdPartyLlmPanel,
+             "ThirdPartyLlmPanel",
+             base::FEATURE_ENABLED_BY_DEFAULT);
+
 BASE_FEATURE(kTabDuplicateMetrics, base::FEATURE_ENABLED_BY_DEFAULT);
 
 // Enables collapsing a tab group programmatically during a drag.
