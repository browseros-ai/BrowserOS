diff --git a/chrome/common/pref_names.h b/chrome/common/pref_names.h
index 7f2ccebd7e50150e350e6a01f7bba9a6796f9286..3be60cfa56f71a3b82727cf2b51420f8085218c5 100644
--- a/chrome/common/pref_names.h
+++ b/chrome/common/pref_names.h
@@ -482,13 +482,11 @@ inline constexpr char kDeskAPIThirdPartyAllowlist[] =
 inline constexpr char kPrintingAPIExtensionsAllowlist[] =
     "printing.printing_api_extensions_whitelist";
 
-
 // A boolean specifying whether the insights extension is enabled. If set to
 // true, the CCaaS Chrome component extension will be installed.
 inline constexpr char kInsightsExtensionEnabled[] =
     "insights_extension_enabled";
 
-
 // A boolean pref which turns on the mediaplayer.
 inline constexpr char kLabsMediaplayerEnabled[] = "settings.labs.mediaplayer";
 
@@ -496,8 +494,6 @@ inline constexpr char kLabsMediaplayerEnabled[] = "settings.labs.mediaplayer";
 inline constexpr char kChromeOSReleaseNotesVersion[] =
     "settings.release_notes.version";
 
-
-
 // A boolean pref. If set to true, the Unified Desktop feature is made
 // available and turned on by default, which allows applications to span
 // multiple screens. Users may turn the feature off and on in the settings
@@ -534,7 +530,6 @@ inline constexpr char kMinimumAllowedChromeVersion[] = "minimum_req.version";
 inline constexpr char kShowArcSettingsOnSessionStart[] =
     "start_arc_settings_on_session_start";
 
-
 // Dictionary preference that maps language to default voice name preferences
 // for the users's text-to-speech settings. For example, this might map
 // 'en-US' to 'Chrome OS US English'.
@@ -556,7 +551,6 @@ inline constexpr char kTextToSpeechPitch[] = "settings.tts.speech_pitch";
 // system volume, and higher than 1.0 is louder.
 inline constexpr char kTextToSpeechVolume[] = "settings.tts.speech_volume";
 
-
 // A string pref storing the path of device wallpaper image file.
 inline constexpr char kDeviceWallpaperImageFilePath[] =
     "policy.device_wallpaper_image_file_path";
@@ -928,6 +922,8 @@ inline constexpr char kImportDialogSavedPasswords[] =
     "import_dialog_saved_passwords";
 inline constexpr char kImportDialogSearchEngine[] =
     "import_dialog_search_engine";
+inline constexpr char kImportDialogExtensions[] = "import_dialog_extensions";
+inline constexpr char kImportDialogCookies[] = "import_dialog_cookies";
 
 // Profile avatar and name
 inline constexpr char kProfileAvatarIndex[] = "profile.avatar_index";
@@ -1232,13 +1228,6 @@ inline constexpr char kProjectsPanelPinnedToTabstrip[] =
 inline constexpr char kEverythingMenuPinnedToTabstrip[] =
     "everything_menu.pinned_to_tabstrip";
 
-// Boolean indicating whether the one-time migration for
-// kEverythingMenuPinnedToTabstrip has been completed. This sets the pinned
-// state for the button to true for users who have used vertical tab strip
-// before the migration happened.
-inline constexpr char kEverythingMenuPinnedToTabstripMigrationComplete[] =
-    "everything_menu.pinned_to_tabstrip_migration_complete";
-
 // Boolean determining whether vertical tabs are enabled.
 inline constexpr char kVerticalTabsEnabled[] = "vertical_tabs.enabled";
 
@@ -2274,12 +2263,6 @@ inline constexpr char kDeviceRobotAnyApiRefreshTokenV2[] =
 inline constexpr char kDeviceRefreshTokenAnyApiIsV3Used[] =
     "device_refresh_token_is_v3_used.any-api";
 
-
-
-
-
-
-
 #endif  // BUILDFLAG(IS_CHROMEOS)
 
 // String which specifies where to store the disk cache.
@@ -2287,7 +2270,6 @@ inline constexpr char kDiskCacheDir[] = "browser.disk_cache_dir";
 // Pref name for the policy specifying the maximal cache size.
 inline constexpr char kDiskCacheSize[] = "browser.disk_cache_size";
 
-
 // Pref name for the policy controlling whether to enable Media Router.
 inline constexpr char kEnableMediaRouter[] = "media_router.enable_media_router";
 #if !BUILDFLAG(IS_ANDROID)
@@ -2423,7 +2405,6 @@ inline constexpr char kPreviousIsolationState[] = "isolation_state.previous";
 inline constexpr char kHardwareAccelerationModePrevious[] =
     "hardware_acceleration_mode_previous";
 
-
 #if !BUILDFLAG(IS_ANDROID)
 // A boolean where true means that the browser has previously attempted to
 // enable autoupdate and failed, so the next out-of-date browser start should
@@ -2958,7 +2939,6 @@ inline constexpr char kOriginAgentClusterDefaultEnabled[] =
 inline constexpr char kSCTAuditingHashdanceReportCount[] =
     "sct_auditing.hashdance_report_count";
 
-
 #if !BUILDFLAG(IS_ANDROID)
 // An integer count of how many times the user has seen the memory saver mode
 // page action chip in the expanded size. While the feature was renamed to
@@ -3262,6 +3242,18 @@ inline constexpr char kCpuPerformanceTierOverride[] =
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
