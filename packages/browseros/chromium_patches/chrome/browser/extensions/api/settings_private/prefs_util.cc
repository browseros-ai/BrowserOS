diff --git a/chrome/browser/extensions/api/settings_private/prefs_util.cc b/chrome/browser/extensions/api/settings_private/prefs_util.cc
index 97bb4be60af93..70bf4e91d6ace 100644
--- a/chrome/browser/extensions/api/settings_private/prefs_util.cc
+++ b/chrome/browser/extensions/api/settings_private/prefs_util.cc
@@ -580,6 +580,11 @@ const PrefsUtil::TypedPrefMap& PrefsUtil::GetAllowlistedKeys() {
   (*s_allowlist)[::prefs::kCaretBrowsingEnabled] =
       settings_api::PrefType::kBoolean;
 
+  // BlockBrowser preferences
+  (*s_allowlist)[prefs::kBlockBrowserProviders] = settings_api::PrefType::kString;
+  (*s_allowlist)[prefs::kBlockBrowserShowToolbarLabels] = settings_api::PrefType::kBoolean;
+  (*s_allowlist)[prefs::kBlockBrowserCustomProviders] = settings_api::PrefType::kString;
+
 #if BUILDFLAG(IS_CHROMEOS)
   // Accounts / Users / People.
   (*s_allowlist)[ash::kAccountsPrefAllowGuest] =
@@ -1164,6 +1169,8 @@ const PrefsUtil::TypedPrefMap& PrefsUtil::GetAllowlistedKeys() {
       settings_api::PrefType::kBoolean;
   (*s_allowlist)[::prefs::kImportDialogSearchEngine] =
       settings_api::PrefType::kBoolean;
+  (*s_allowlist)[::prefs::kImportDialogExtensions] =
+      settings_api::PrefType::kBoolean;
 #endif  // BUILDFLAG(IS_CHROMEOS)
 
   // Supervised Users.  This setting is queried in our Tast tests (b/241943380).
