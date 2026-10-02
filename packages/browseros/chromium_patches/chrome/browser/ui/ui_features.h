diff --git a/chrome/browser/ui/ui_features.h b/chrome/browser/ui/ui_features.h
index 5de002c32e2fc902ccf37d4251fb479bc3b6b579..5bfd297d71c43b4cdbc0913a2093a4e2c3d735fc 100644
--- a/chrome/browser/ui/ui_features.h
+++ b/chrome/browser/ui/ui_features.h
@@ -202,6 +202,9 @@ BASE_DECLARE_FEATURE_PARAM(base::TimeDelta, kSplitViewDragAndDropMaxDelay);
 BASE_DECLARE_FEATURE_PARAM(int, kSplitViewDragAndDropMinDistanceThreshold);
 BASE_DECLARE_FEATURE_PARAM(int, kSplitViewDragAndDropMaxDistanceThreshold);
 
+// BrowserOS: feature declarations
+BASE_DECLARE_FEATURE(kThirdPartyLlmPanel);
+
 BASE_DECLARE_FEATURE(kTabDuplicateMetrics);
 
 BASE_DECLARE_FEATURE(kCollapseTabGroupDuringDrag);
