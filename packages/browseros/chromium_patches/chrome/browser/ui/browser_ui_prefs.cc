diff --git a/chrome/browser/ui/browser_ui_prefs.cc b/chrome/browser/ui/browser_ui_prefs.cc
index e94568b1c7542..25e7e791009a0 100644
--- a/chrome/browser/ui/browser_ui_prefs.cc
+++ b/chrome/browser/ui/browser_ui_prefs.cc
@@ -68,7 +68,7 @@ void RegisterBrowserPrefs(PrefRegistrySimple* registry) {
 
   registry->RegisterBooleanPref(prefs::kHoverCardImagesEnabled, true);
 
-  registry->RegisterBooleanPref(prefs::kHoverCardMemoryUsageEnabled, true);
+  registry->RegisterBooleanPref(prefs::kHoverCardMemoryUsageEnabled, false);
 
 #if defined(USE_AURA)
   registry->RegisterBooleanPref(prefs::kOverscrollHistoryNavigationEnabled,
@@ -112,7 +112,7 @@ void RegisterBrowserUserPrefs(user_prefs::PrefRegistrySyncable* registry) {
 
   registry->RegisterBooleanPref(prefs::kHomePageIsNewTabPage, true,
                                 pref_registration_flags);
-  registry->RegisterBooleanPref(prefs::kShowHomeButton, false,
+  registry->RegisterBooleanPref(prefs::kShowHomeButton, true,
                                 pref_registration_flags);
   registry->RegisterBooleanPref(prefs::kSplitViewDragAndDropEnabled, true,
                                 pref_registration_flags);
@@ -121,7 +121,8 @@ void RegisterBrowserUserPrefs(user_prefs::PrefRegistrySyncable* registry) {
                                 pref_registration_flags);
   registry->RegisterBooleanPref(prefs::kPinContextualTaskButton, true,
                                 pref_registration_flags);
-  registry->RegisterBooleanPref(prefs::kPinSplitTabButton, false,
+  // BrowserOS: default split tab button to pinned
+  registry->RegisterBooleanPref(prefs::kPinSplitTabButton, true,
                                 pref_registration_flags);
 
   registry->RegisterInt64Pref(prefs::kDefaultBrowserLastDeclined, 0);
