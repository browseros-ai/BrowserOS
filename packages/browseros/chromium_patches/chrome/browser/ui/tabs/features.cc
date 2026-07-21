diff --git a/chrome/browser/ui/tabs/features.cc b/chrome/browser/ui/tabs/features.cc
index e9704aeea9dacf633d2e6b48fcbd387608911af6..2506eff0f46c3dbc1a35b2138bb6e27706d204cd 100644
--- a/chrome/browser/ui/tabs/features.cc
+++ b/chrome/browser/ui/tabs/features.cc
@@ -26,7 +26,7 @@ BASE_FEATURE(kSplitViewTabRestore, base::FEATURE_DISABLED_BY_DEFAULT);
 
 BASE_FEATURE(kTabSearchCjkWordBoundary, base::FEATURE_DISABLED_BY_DEFAULT);
 
-BASE_FEATURE(kVerticalTabs, base::FEATURE_DISABLED_BY_DEFAULT);
+BASE_FEATURE(kVerticalTabs, base::FEATURE_ENABLED_BY_DEFAULT);
 
 BASE_FEATURE(kVerticalTabsLaunch, base::FEATURE_DISABLED_BY_DEFAULT);
 BASE_FEATURE_PARAM(bool,
