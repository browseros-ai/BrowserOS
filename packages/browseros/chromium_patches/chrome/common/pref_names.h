diff --git a/chrome/common/pref_names.h b/chrome/common/pref_names.h
index 898e1c48db1e1..3f88b936ccd63 100644
--- a/chrome/common/pref_names.h
+++ b/chrome/common/pref_names.h
@@ -1594,6 +1594,9 @@ inline constexpr char kImportDialogSavedPasswords[] =
     "import_dialog_saved_passwords";
 inline constexpr char kImportDialogSearchEngine[] =
     "import_dialog_search_engine";
+inline constexpr char kImportDialogExtensions[] =
+    "import_dialog_extensions";
+inline constexpr char kImportDialogCookies[] = "import_dialog_cookies";
 
 // Profile avatar and name
 inline constexpr char kProfileAvatarIndex[] = "profile.avatar_index";
@@ -4336,6 +4339,18 @@ inline constexpr char kAndroidTipNotificationShownBottomOmnibox[] =
 // LINT.ThenChange(//chrome/android/java/src/org/chromium/chrome/browser/notifications/tips/TipsUtils.java:TipsShownPrefs)
 #endif  // BUILDFLAG(IS_ANDROID)
 
+// BrowserOS: metrics prefs
+// String containing the stable client ID for BrowserOS metrics
+inline constexpr char kBrowserOSMetricsClientId[] =
+    "browseros.metrics_client_id";
+
+// String containing the stable install ID for BrowserOS metrics (Local State)
+inline constexpr char kBrowserOSMetricsInstallId[] =
+    "browseros.metrics_install_id";
+
+// NOTE: Other BrowserOS prefs have been moved to
+// chrome/browser/browseros/core/browseros_prefs.h
+
 }  // namespace prefs
 
 #endif  // CHROME_COMMON_PREF_NAMES_H_
