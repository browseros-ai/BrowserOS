diff --git a/chrome/browser/ui/ui_features.cc b/chrome/browser/ui/ui_features.cc
index 8e70b4d6c09a8..f63de4c04083c 100644
--- a/chrome/browser/ui/ui_features.cc
+++ b/chrome/browser/ui/ui_features.cc
@@ -115,6 +115,14 @@ BASE_FEATURE(kPopupBrowserUseNewLayout, base::FEATURE_ENABLED_BY_DEFAULT);
 
 BASE_FEATURE(kTabbedBrowserUseNewLayout, base::FEATURE_ENABLED_BY_DEFAULT);
 
+BASE_FEATURE(kThirdPartyLlmPanel,
+             "ThirdPartyLlmPanel",
+             base::FEATURE_ENABLED_BY_DEFAULT);
+
+BASE_FEATURE(kClashOfGpts,
+             "ClashOfGpts",
+             base::FEATURE_ENABLED_BY_DEFAULT);
+
 BASE_FEATURE(kTabDuplicateMetrics, base::FEATURE_ENABLED_BY_DEFAULT);
 
 // Enables tabs to be frozen when collapsed.
