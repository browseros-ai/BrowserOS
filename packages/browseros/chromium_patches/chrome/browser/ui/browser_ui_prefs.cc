diff --git a/chrome/browser/ui/browser_ui_prefs.cc b/chrome/browser/ui/browser_ui_prefs.cc
index 25d238a208dcf..4700b8fdfcea0 100644
--- a/chrome/browser/ui/browser_ui_prefs.cc
+++ b/chrome/browser/ui/browser_ui_prefs.cc
@@ -68,7 +68,7 @@ void RegisterBrowserPrefs(PrefRegistrySimple* registry) {
 
   registry->RegisterBooleanPref(prefs::kHoverCardImagesEnabled, true);
 
-  registry->RegisterBooleanPref(prefs::kHoverCardMemoryUsageEnabled, true);
+  registry->RegisterBooleanPref(prefs::kHoverCardMemoryUsageEnabled, false);
 
 #if defined(USE_AURA)
   registry->RegisterBooleanPref(prefs::kOverscrollHistoryNavigationEnabled,
@@ -115,7 +115,7 @@ void RegisterBrowserUserPrefs(user_prefs::PrefRegistrySyncable* registry) {
 
   registry->RegisterBooleanPref(prefs::kHomePageIsNewTabPage, true,
                                 pref_registration_flags);
-  registry->RegisterBooleanPref(prefs::kShowHomeButton, false,
+  registry->RegisterBooleanPref(prefs::kShowHomeButton, true,
                                 pref_registration_flags);
   registry->RegisterBooleanPref(prefs::kSplitViewDragAndDropEnabled, true,
                                 pref_registration_flags);
@@ -126,7 +126,8 @@ void RegisterBrowserUserPrefs(user_prefs::PrefRegistrySyncable* registry) {
   registry->RegisterInt64Pref(prefs::kBookmarkBarNavigationCount, 0);
   registry->RegisterBooleanPref(prefs::kPinContextualTaskButton, true,
                                 pref_registration_flags);
-  registry->RegisterBooleanPref(prefs::kPinSplitTabButton, false,
+  // BrowserOS: default split tab button to pinned
+  registry->RegisterBooleanPref(prefs::kPinSplitTabButton, true,
                                 pref_registration_flags);
 
   registry->RegisterInt64Pref(prefs::kDefaultBrowserInfobarLastDeclined, 0);
