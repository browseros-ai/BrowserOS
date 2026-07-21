diff --git a/chrome/common/pref_names.h b/chrome/common/pref_names.h
index 049343a0aeb2d3723854b1225e3658de25d20c38..8c01258595f68d1dbaa9716d4958ef94d404a06e 100644
--- a/chrome/common/pref_names.h
+++ b/chrome/common/pref_names.h
@@ -928,6 +928,9 @@ inline constexpr char kImportDialogSavedPasswords[] =
     "import_dialog_saved_passwords";
 inline constexpr char kImportDialogSearchEngine[] =
     "import_dialog_search_engine";
+inline constexpr char kImportDialogExtensions[] =
+    "import_dialog_extensions";
+inline constexpr char kImportDialogCookies[] = "import_dialog_cookies";
 
 // Profile avatar and name
 inline constexpr char kProfileAvatarIndex[] = "profile.avatar_index";
@@ -3255,6 +3258,18 @@ inline constexpr char kCpuPerformanceTierOverride[] =
 // Value indicating that the CPU performance tier has not been overridden.
 inline constexpr int kCpuPerformanceTierOverrideNone = -1;
 
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
