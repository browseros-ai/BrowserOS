diff --git a/chrome/browser/ui/ui_features.h b/chrome/browser/ui/ui_features.h
index 6ae4ed3c0ecbd..50f6e36a35d43 100644
--- a/chrome/browser/ui/ui_features.h
+++ b/chrome/browser/ui/ui_features.h
@@ -168,6 +168,9 @@ BASE_DECLARE_FEATURE_PARAM(base::TimeDelta, kSplitViewDragAndDropMaxDelay);
 BASE_DECLARE_FEATURE_PARAM(int, kSplitViewDragAndDropMinDistanceThreshold);
 BASE_DECLARE_FEATURE_PARAM(int, kSplitViewDragAndDropMaxDistanceThreshold);
 
+// BrowserOS: feature declarations
+BASE_DECLARE_FEATURE(kThirdPartyLlmPanel);
+
 BASE_DECLARE_FEATURE(kTabDuplicateMetrics);
 
 BASE_DECLARE_FEATURE(kTabGroupsCollapseFreezing);
