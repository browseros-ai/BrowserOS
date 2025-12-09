diff --git a/chrome/browser/ui/browser_ui_prefs.cc b/chrome/browser/ui/browser_ui_prefs.cc
index 7a2f76324af50..d87b0e71d9a98 100644
--- a/chrome/browser/ui/browser_ui_prefs.cc
+++ b/chrome/browser/ui/browser_ui_prefs.cc
@@ -65,7 +65,7 @@ void RegisterBrowserPrefs(PrefRegistrySimple* registry) {
 
   registry->RegisterBooleanPref(prefs::kHoverCardImagesEnabled, true);
 
-  registry->RegisterBooleanPref(prefs::kHoverCardMemoryUsageEnabled, true);
+  registry->RegisterBooleanPref(prefs::kHoverCardMemoryUsageEnabled, false);
 
 #if defined(USE_AURA)
   registry->RegisterBooleanPref(prefs::kOverscrollHistoryNavigationEnabled,
@@ -109,7 +109,7 @@ void RegisterBrowserUserPrefs(user_prefs::PrefRegistrySyncable* registry) {
 
   registry->RegisterBooleanPref(prefs::kHomePageIsNewTabPage, true,
                                 pref_registration_flags);
-  registry->RegisterBooleanPref(prefs::kShowHomeButton, false,
+  registry->RegisterBooleanPref(prefs::kShowHomeButton, true,
                                 pref_registration_flags);
   registry->RegisterBooleanPref(prefs::kSplitViewDragAndDropEnabled, true,
                                 pref_registration_flags);
@@ -117,7 +117,7 @@ void RegisterBrowserUserPrefs(user_prefs::PrefRegistrySyncable* registry) {
   registry->RegisterBooleanPref(prefs::kShowForwardButton, true,
                                 pref_registration_flags);
 
-  registry->RegisterBooleanPref(prefs::kPinSplitTabButton, false,
+  registry->RegisterBooleanPref(prefs::kPinSplitTabButton, true,
                                 pref_registration_flags);
 
   registry->RegisterInt64Pref(prefs::kDefaultBrowserLastDeclined, 0);
