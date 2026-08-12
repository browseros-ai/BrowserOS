diff --git a/chrome/browser/ui/tabs/features.cc b/chrome/browser/ui/tabs/features.cc
index 483be333e73a5432998b142691a1e4f2353c4e95..71ed3bfbe0ebecd4f46d690360f41906a6e85c72 100644
--- a/chrome/browser/ui/tabs/features.cc
+++ b/chrome/browser/ui/tabs/features.cc
@@ -26,7 +26,7 @@ BASE_FEATURE(kSplitViewTabRestore, base::FEATURE_DISABLED_BY_DEFAULT);
 
 BASE_FEATURE(kTabSearchCjkWordBoundary, base::FEATURE_DISABLED_BY_DEFAULT);
 
-BASE_FEATURE(kVerticalTabs, base::FEATURE_DISABLED_BY_DEFAULT);
+BASE_FEATURE(kVerticalTabs, base::FEATURE_ENABLED_BY_DEFAULT);
 
 BASE_FEATURE(kVerticalTabsLaunch, base::FEATURE_DISABLED_BY_DEFAULT);
 BASE_FEATURE_PARAM(bool,
