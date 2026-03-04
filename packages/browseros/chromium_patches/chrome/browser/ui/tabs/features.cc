diff --git a/chrome/browser/ui/tabs/features.cc b/chrome/browser/ui/tabs/features.cc
index dfe2538c493c1..6e8685ee634f1 100644
--- a/chrome/browser/ui/tabs/features.cc
+++ b/chrome/browser/ui/tabs/features.cc
@@ -15,7 +15,7 @@ BASE_FEATURE(kDebugUITabStrip, base::FEATURE_DISABLED_BY_DEFAULT);
 
 BASE_FEATURE(kTabGroupHome, base::FEATURE_DISABLED_BY_DEFAULT);
 
-BASE_FEATURE(kVerticalTabs, base::FEATURE_DISABLED_BY_DEFAULT);
+BASE_FEATURE(kVerticalTabs, base::FEATURE_ENABLED_BY_DEFAULT);
 
 BASE_FEATURE(kTabSelectionByPointer, base::FEATURE_ENABLED_BY_DEFAULT);
 
