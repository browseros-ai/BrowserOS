diff --git a/chrome/browser/ui/ui_features.h b/chrome/browser/ui/ui_features.h
index 43f876760f130..a1ed839eae250 100644
--- a/chrome/browser/ui/ui_features.h
+++ b/chrome/browser/ui/ui_features.h
@@ -156,6 +156,10 @@ BASE_DECLARE_FEATURE(kSideBySide);
 
 BASE_DECLARE_FEATURE(kSideBySideLinkMenuNewBadge);
 
+// BrowserOS: feature declarations
+BASE_DECLARE_FEATURE(kThirdPartyLlmPanel);
+BASE_DECLARE_FEATURE(kClashOfGpts);
+
 BASE_DECLARE_FEATURE(kTabDuplicateMetrics);
 
 BASE_DECLARE_FEATURE(kTabGroupsCollapseFreezing);
