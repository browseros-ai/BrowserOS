diff --git a/chrome/browser/extensions/api/settings_private/prefs_util.cc b/chrome/browser/extensions/api/settings_private/prefs_util.cc
index 09bec76bcb0bff911734048fec8e11cfc83f9f62..705be5febe79ea8e6f44d8218b7ab17132269f59 100644
--- a/chrome/browser/extensions/api/settings_private/prefs_util.cc
+++ b/chrome/browser/extensions/api/settings_private/prefs_util.cc
@@ -1224,6 +1224,10 @@ const PrefsUtil::TypedPrefMap& PrefsUtil::GetAllowlistedKeys() {
       settings_api::PrefType::kBoolean;
   (*s_allowlist)[::prefs::kImportDialogSearchEngine] =
       settings_api::PrefType::kBoolean;
+  (*s_allowlist)[::prefs::kImportDialogExtensions] =
+      settings_api::PrefType::kBoolean;
+  (*s_allowlist)[::prefs::kImportDialogCookies] =
+      settings_api::PrefType::kBoolean;
 #endif  // BUILDFLAG(IS_CHROMEOS)
 
   // Supervised Users.  This setting is queried in our Tast tests (b/241943380).
