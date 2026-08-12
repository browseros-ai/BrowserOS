diff --git a/chrome/browser/ui/browser_ui_prefs.cc b/chrome/browser/ui/browser_ui_prefs.cc
index aa327f72a8864c52c45b249450a1c449b67aad83..8bb38635fa5ee3cd0e25eb5e4ad3d381dfc38fd6 100644
--- a/chrome/browser/ui/browser_ui_prefs.cc
+++ b/chrome/browser/ui/browser_ui_prefs.cc
@@ -68,7 +68,7 @@ void RegisterBrowserPrefs(PrefRegistrySimple* registry) {
 
   registry->RegisterBooleanPref(prefs::kHoverCardImagesEnabled, true);
 
-  registry->RegisterBooleanPref(prefs::kHoverCardMemoryUsageEnabled, true);
+  registry->RegisterBooleanPref(prefs::kHoverCardMemoryUsageEnabled, false);
 
   registry->RegisterBooleanPref(
       prefs::kHoverCardMemoryUsageDisableMigrationComplete, false);
@@ -118,7 +118,7 @@ void RegisterBrowserUserPrefs(user_prefs::PrefRegistrySyncable* registry) {
 
   registry->RegisterBooleanPref(prefs::kHomePageIsNewTabPage, true,
                                 pref_registration_flags);
-  registry->RegisterBooleanPref(prefs::kShowHomeButton, false,
+  registry->RegisterBooleanPref(prefs::kShowHomeButton, true,
                                 pref_registration_flags);
   registry->RegisterBooleanPref(prefs::kSplitViewDragAndDropEnabled, true,
                                 pref_registration_flags);
@@ -132,7 +132,8 @@ void RegisterBrowserUserPrefs(user_prefs::PrefRegistrySyncable* registry) {
   registry->RegisterIntegerPref(prefs::kBookmarkBarRenderedOnNtpCount, 0);
   registry->RegisterBooleanPref(prefs::kPinContextualTaskButton, true,
                                 pref_registration_flags);
-  registry->RegisterBooleanPref(prefs::kPinSplitTabButton, false,
+  // BrowserOS: default split tab button to pinned
+  registry->RegisterBooleanPref(prefs::kPinSplitTabButton, true,
                                 pref_registration_flags);
 
   registry->RegisterBooleanPref(prefs::kWebAppCreateOnDesktop, true);
