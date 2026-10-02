diff --git a/chrome/common/pref_names.h b/chrome/common/pref_names.h
index a4b175a64f5a58eec344df0b38c49df459f16ba7..6c0ed877f379d0190bdf2d0f62d9efd703e4a704 100644
--- a/chrome/common/pref_names.h
+++ b/chrome/common/pref_names.h
@@ -485,13 +485,11 @@ inline constexpr char kDeskAPIThirdPartyAllowlist[] =
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
 
@@ -499,8 +497,6 @@ inline constexpr char kLabsMediaplayerEnabled[] = "settings.labs.mediaplayer";
 inline constexpr char kChromeOSReleaseNotesVersion[] =
     "settings.release_notes.version";
 
-
-
 // A boolean pref. If set to true, the Unified Desktop feature is made
 // available and turned on by default, which allows applications to span
 // multiple screens. Users may turn the feature off and on in the settings
@@ -537,7 +533,6 @@ inline constexpr char kMinimumAllowedChromeVersion[] = "minimum_req.version";
 inline constexpr char kShowArcSettingsOnSessionStart[] =
     "start_arc_settings_on_session_start";
 
-
 // Dictionary preference that maps language to default voice name preferences
 // for the users's text-to-speech settings. For example, this might map
 // 'en-US' to 'Chrome OS US English'.
@@ -559,7 +554,6 @@ inline constexpr char kTextToSpeechPitch[] = "settings.tts.speech_pitch";
 // system volume, and higher than 1.0 is louder.
 inline constexpr char kTextToSpeechVolume[] = "settings.tts.speech_volume";
 
-
 // A string pref storing the path of device wallpaper image file.
 inline constexpr char kDeviceWallpaperImageFilePath[] =
     "policy.device_wallpaper_image_file_path";
@@ -940,6 +934,8 @@ inline constexpr char kImportDialogSavedPasswords[] =
     "import_dialog_saved_passwords";
 inline constexpr char kImportDialogSearchEngine[] =
     "import_dialog_search_engine";
+inline constexpr char kImportDialogExtensions[] = "import_dialog_extensions";
+inline constexpr char kImportDialogCookies[] = "import_dialog_cookies";
 
 // Profile avatar and name
 inline constexpr char kProfileAvatarIndex[] = "profile.avatar_index";
@@ -2298,12 +2294,6 @@ inline constexpr char kDeviceRobotAnyApiRefreshTokenV2[] =
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
@@ -2311,7 +2301,6 @@ inline constexpr char kDiskCacheDir[] = "browser.disk_cache_dir";
 // Pref name for the policy specifying the maximal cache size.
 inline constexpr char kDiskCacheSize[] = "browser.disk_cache_size";
 
-
 // Pref name for the policy controlling whether to enable Media Router.
 inline constexpr char kEnableMediaRouter[] = "media_router.enable_media_router";
 #if !BUILDFLAG(IS_ANDROID)
@@ -2447,7 +2436,6 @@ inline constexpr char kPreviousIsolationState[] = "isolation_state.previous";
 inline constexpr char kHardwareAccelerationModePrevious[] =
     "hardware_acceleration_mode_previous";
 
-
 #if !BUILDFLAG(IS_ANDROID)
 // A boolean where true means that the browser has previously attempted to
 // enable autoupdate and failed, so the next out-of-date browser start should
@@ -2987,7 +2975,6 @@ inline constexpr char kOriginAgentClusterDefaultEnabled[] =
 inline constexpr char kSCTAuditingHashdanceReportCount[] =
     "sct_auditing.hashdance_report_count";
 
-
 #if !BUILDFLAG(IS_ANDROID)
 // An integer count of how many times the user has seen the memory saver mode
 // page action chip in the expanded size. While the feature was renamed to
@@ -3296,6 +3283,9 @@ inline constexpr char kCpuPerformanceTierOverride[] =
 // Value indicating that the CPU performance tier has not been overridden.
 inline constexpr int kCpuPerformanceTierOverrideNone = -1;
 
+// NOTE: Other BrowserOS prefs have been moved to
+// chrome/browser/browseros/core/browseros_prefs.h
+
 }  // namespace prefs
 
 #endif  // CHROME_COMMON_PREF_NAMES_H_
