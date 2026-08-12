diff --git a/chrome/browser/ui/ui_features.cc b/chrome/browser/ui/ui_features.cc
index e7c8cc09dd4ebadf51750bed8e51ec9af90dc71a..1fd66305700b0f038adef745da1178634be29217 100644
--- a/chrome/browser/ui/ui_features.cc
+++ b/chrome/browser/ui/ui_features.cc
@@ -122,7 +122,7 @@ BASE_FEATURE(kExtensionsPinnedByDefault, base::FEATURE_DISABLED_BY_DEFAULT);
 #if BUILDFLAG(IS_WIN) || BUILDFLAG(IS_MAC)
 // Shows an infobar on PDFs offering to become the default PDF viewer if Chrome
 // isn't the default already.
-BASE_FEATURE(kPdfInfoBar, base::FEATURE_ENABLED_BY_DEFAULT);
+BASE_FEATURE(kPdfInfoBar, base::FEATURE_DISABLED_BY_DEFAULT);
 
 BASE_FEATURE(kSeparateDefaultAndPinPrompt, base::FEATURE_DISABLED_BY_DEFAULT);
 BASE_FEATURE_PARAM(int,
@@ -219,6 +219,10 @@ BASE_FEATURE_PARAM(int,
                    "max_distance_threshold",
                    20);
 
+BASE_FEATURE(kThirdPartyLlmPanel,
+             "ThirdPartyLlmPanel",
+             base::FEATURE_ENABLED_BY_DEFAULT);
+
 BASE_FEATURE(kTabDuplicateMetrics, base::FEATURE_ENABLED_BY_DEFAULT);
 
 // Enables tabs to be frozen when collapsed.
