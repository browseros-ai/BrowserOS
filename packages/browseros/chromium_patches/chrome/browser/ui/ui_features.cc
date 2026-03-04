diff --git a/chrome/browser/ui/ui_features.cc b/chrome/browser/ui/ui_features.cc
index 45501ce4b433d..347348bcc362d 100644
--- a/chrome/browser/ui/ui_features.cc
+++ b/chrome/browser/ui/ui_features.cc
@@ -126,6 +126,14 @@ BASE_FEATURE(kSideBySide, base::FEATURE_ENABLED_BY_DEFAULT);
 
 BASE_FEATURE(kSideBySideLinkMenuNewBadge, base::FEATURE_ENABLED_BY_DEFAULT);
 
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
