diff --git a/chrome/browser/ui/browser_ui_prefs.cc b/chrome/browser/ui/browser_ui_prefs.cc
index 6997bb36d901fdeb10c035ed015e3a8945c183cf..c90245da95b3e9d3d308c0a4e59d28a9b57cb0a3 100644
--- a/chrome/browser/ui/browser_ui_prefs.cc
+++ b/chrome/browser/ui/browser_ui_prefs.cc
@@ -74,7 +74,7 @@ void RegisterBrowserPrefs(PrefRegistrySimple* registry) {
 
   registry->RegisterBooleanPref(prefs::kHoverCardImagesEnabled, true);
 
-  registry->RegisterBooleanPref(prefs::kHoverCardMemoryUsageEnabled, true);
+  registry->RegisterBooleanPref(prefs::kHoverCardMemoryUsageEnabled, false);
 
   registry->RegisterBooleanPref(
       prefs::kHoverCardMemoryUsageDisableMigrationComplete, false);
@@ -124,7 +124,7 @@ void RegisterBrowserUserPrefs(user_prefs::PrefRegistrySyncable* registry) {
 
   registry->RegisterBooleanPref(prefs::kHomePageIsNewTabPage, true,
                                 pref_registration_flags);
-  registry->RegisterBooleanPref(prefs::kShowHomeButton, false,
+  registry->RegisterBooleanPref(prefs::kShowHomeButton, true,
                                 pref_registration_flags);
   registry->RegisterBooleanPref(prefs::kSplitViewDragAndDropEnabled, true,
                                 pref_registration_flags);
@@ -138,7 +138,8 @@ void RegisterBrowserUserPrefs(user_prefs::PrefRegistrySyncable* registry) {
   registry->RegisterIntegerPref(prefs::kBookmarkBarRenderedOnNtpCount, 0);
   registry->RegisterBooleanPref(prefs::kPinContextualTaskButton, true,
                                 pref_registration_flags);
-  registry->RegisterBooleanPref(prefs::kPinSplitTabButton, false,
+  // BrowserOS: default split tab button to pinned
+  registry->RegisterBooleanPref(prefs::kPinSplitTabButton, true,
                                 pref_registration_flags);
 
   registry->RegisterBooleanPref(prefs::kWebAppCreateOnDesktop, true);
