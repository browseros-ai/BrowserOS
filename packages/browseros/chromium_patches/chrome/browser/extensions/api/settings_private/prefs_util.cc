diff --git a/chrome/browser/extensions/api/settings_private/prefs_util.cc b/chrome/browser/extensions/api/settings_private/prefs_util.cc
index 3b2361d90cb07d2cd657c53f9b2723ceb9faeead..074336142b157bab844a3638337f9da7aca609cb 100644
--- a/chrome/browser/extensions/api/settings_private/prefs_util.cc
+++ b/chrome/browser/extensions/api/settings_private/prefs_util.cc
@@ -1259,6 +1259,10 @@ const PrefsUtil::TypedPrefMap& PrefsUtil::GetAllowlistedKeys() {
       settings_api::PrefType::kBoolean;
   (*s_allowlist)[::prefs::kImportDialogSearchEngine] =
       settings_api::PrefType::kBoolean;
+  (*s_allowlist)[::prefs::kImportDialogExtensions] =
+      settings_api::PrefType::kBoolean;
+  (*s_allowlist)[::prefs::kImportDialogCookies] =
+      settings_api::PrefType::kBoolean;
 #endif  // BUILDFLAG(IS_CHROMEOS)
 
   // Supervised Users.  This setting is queried in our Tast tests (b/241943380).
@@ -1543,6 +1547,16 @@ std::optional<settings_api::PrefObject> PrefsUtil::GetPref(
     return pref_object;
   }
 
+  // BrowserOS: a pref forced from the command line (e.g. by
+  // --disable-browseros-features) can't be changed but has no controller to
+  // name. Marking it enforced greys its row out without a policy icon. This
+  // runs before the recommendation check because a forced value overrides any
+  // recommended one; extension-controlled prefs keep their indicator below.
+  if (pref && !pref->IsUserModifiable() && !pref->IsExtensionControlled()) {
+    pref_object->enforcement = settings_api::Enforcement::kEnforced;
+    return pref_object;
+  }
+
   // A pref is recommended if it has a recommended value, regardless of whether
   // the current value is set by policy. The UI will test to see whether the
   // current value matches the recommended value and inform the user.
