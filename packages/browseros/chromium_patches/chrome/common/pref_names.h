diff --git a/chrome/common/pref_names.h b/chrome/common/pref_names.h
index 1a4683393ff24..dc1e69f5fc57d 100644
--- a/chrome/common/pref_names.h
+++ b/chrome/common/pref_names.h
@@ -1583,6 +1583,9 @@ inline constexpr char kImportDialogSavedPasswords[] =
     "import_dialog_saved_passwords";
 inline constexpr char kImportDialogSearchEngine[] =
     "import_dialog_search_engine";
+inline constexpr char kImportDialogExtensions[] =
+    "import_dialog_extensions";
+inline constexpr char kImportDialogCookies[] = "import_dialog_cookies";
 
 // Profile avatar and name
 inline constexpr char kProfileAvatarIndex[] = "profile.avatar_index";
@@ -4302,6 +4305,17 @@ inline constexpr char kNonMilestoneUpdateToastVersion[] =
     "toast.non_milestone_update_toast_version";
 #endif  // !BUILDFLAG(IS_ANDROID)
 
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
