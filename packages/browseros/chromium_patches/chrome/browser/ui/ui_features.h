diff --git a/chrome/browser/ui/ui_features.h b/chrome/browser/ui/ui_features.h
index 5588ad208df61dfca91caef030c2cee96fc1d818..4f539a50a548d721b7550a78ede4750713af6fe9 100644
--- a/chrome/browser/ui/ui_features.h
+++ b/chrome/browser/ui/ui_features.h
@@ -187,6 +187,9 @@ BASE_DECLARE_FEATURE_PARAM(base::TimeDelta, kSplitViewDragAndDropMaxDelay);
 BASE_DECLARE_FEATURE_PARAM(int, kSplitViewDragAndDropMinDistanceThreshold);
 BASE_DECLARE_FEATURE_PARAM(int, kSplitViewDragAndDropMaxDistanceThreshold);
 
+// BrowserOS: feature declarations
+BASE_DECLARE_FEATURE(kThirdPartyLlmPanel);
+
 BASE_DECLARE_FEATURE(kTabDuplicateMetrics);
 
 BASE_DECLARE_FEATURE(kTabGroupsCollapseFreezing);
