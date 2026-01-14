diff --git a/chrome/browser/extensions/api/settings_private/prefs_util.cc b/chrome/browser/extensions/api/settings_private/prefs_util.cc
index 79c4eb8fc905c..dd7f36f954fb1 100644
--- a/chrome/browser/extensions/api/settings_private/prefs_util.cc
+++ b/chrome/browser/extensions/api/settings_private/prefs_util.cc
@@ -14,6 +14,7 @@
 #include "chrome/browser/accessibility/tree_fixing/pref_names.h"
 #include "chrome/browser/browser_process.h"
 #include "chrome/browser/browser_process_platform_part.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
 #include "chrome/browser/content_settings/generated_cookie_prefs.h"
 #include "chrome/browser/content_settings/generated_javascript_optimizer_pref.h"
 #include "chrome/browser/content_settings/generated_permission_prompting_behavior_pref.h"
@@ -592,6 +593,18 @@ const PrefsUtil::TypedPrefMap& PrefsUtil::GetAllowlistedKeys() {
   (*s_allowlist)[::prefs::kCaretBrowsingEnabled] =
       settings_api::PrefType::kBoolean;
 
+  // BrowserOS prefs (all in browseros::prefs namespace)
+  (*s_allowlist)[browseros::prefs::kProviders] =
+      settings_api::PrefType::kString;
+  (*s_allowlist)[browseros::prefs::kCustomProviders] =
+      settings_api::PrefType::kString;
+  (*s_allowlist)[browseros::prefs::kShowToolbarLabels] =
+      settings_api::PrefType::kBoolean;
+  (*s_allowlist)[browseros::prefs::kShowLLMChat] =
+      settings_api::PrefType::kBoolean;
+  (*s_allowlist)[browseros::prefs::kShowLLMHub] =
+      settings_api::PrefType::kBoolean;
+
 #if BUILDFLAG(IS_CHROMEOS)
   // Accounts / Users / People.
   (*s_allowlist)[ash::kAccountsPrefAllowGuest] =
@@ -1180,6 +1193,10 @@ const PrefsUtil::TypedPrefMap& PrefsUtil::GetAllowlistedKeys() {
       settings_api::PrefType::kBoolean;
   (*s_allowlist)[::prefs::kImportDialogSearchEngine] =
       settings_api::PrefType::kBoolean;
+  (*s_allowlist)[::prefs::kImportDialogExtensions] =
+      settings_api::PrefType::kBoolean;
+  (*s_allowlist)[::prefs::kImportDialogCookies] =
+      settings_api::PrefType::kBoolean;
 #endif  // BUILDFLAG(IS_CHROMEOS)
 
   // Supervised Users.  This setting is queried in our Tast tests (b/241943380).
