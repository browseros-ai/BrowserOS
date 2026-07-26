diff --git a/chrome/browser/ui/tabs/features.cc b/chrome/browser/ui/tabs/features.cc
index faa16e19c872f..11db930e9341b 100644
--- a/chrome/browser/ui/tabs/features.cc
+++ b/chrome/browser/ui/tabs/features.cc
@@ -15,7 +15,7 @@ BASE_FEATURE(kTabGroupHome, base::FEATURE_DISABLED_BY_DEFAULT);
 BASE_FEATURE(kSessionRestoreShowThrobberOnVisible,
              base::FEATURE_DISABLED_BY_DEFAULT);
 
-BASE_FEATURE(kVerticalTabs, base::FEATURE_DISABLED_BY_DEFAULT);
+BASE_FEATURE(kVerticalTabs, base::FEATURE_ENABLED_BY_DEFAULT);
 
 BASE_FEATURE(kVerticalTabsLaunch, base::FEATURE_DISABLED_BY_DEFAULT);
 BASE_FEATURE_PARAM(bool,
