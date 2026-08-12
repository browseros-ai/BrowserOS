diff --git a/chrome/browser/flag_descriptions.h b/chrome/browser/flag_descriptions.h
index b7f2932b50147bbda41b55ab9072b1e37ce364aa..8c881d78d2bfed4b9f74bc60e4027affb88da87b 100644
--- a/chrome/browser/flag_descriptions.h
+++ b/chrome/browser/flag_descriptions.h
@@ -45,7 +45,6 @@ inline constexpr char kAccelerated2dCanvasDescription[] =
     "Enables the use of the GPU to perform 2d canvas rendering instead of "
     "using software rendering.";
 
-
 inline constexpr char kAiModeEntryPointAlwaysNavigatesName[] =
     "AI Mode Omnibox Entrypoint always navigates";
 inline constexpr char kAiModeEntryPointAlwaysNavigatesDescription[] =
@@ -53,7 +52,7 @@ inline constexpr char kAiModeEntryPointAlwaysNavigatesDescription[] =
     "google.com/aimode.";
 
 inline constexpr char kDynamicAiModeButtonName[] =
-        "Omnibox Dynamic AI Mode Button";
+    "Omnibox Dynamic AI Mode Button";
 inline constexpr char kDynamicAiModeButtonDescription[] =
     "Enables dynamic behaviour for the AI mode button in the WebUI Omnibox.";
 
@@ -325,6 +324,18 @@ inline constexpr char kBlockingFocusWithoutUserActivationDescription[] =
     "(element.focus(), window.focus(), autofocus) from iframes unless "
     "triggered by a user gesture.";
 
+// BrowserOS: feature flags
+inline constexpr char kBrowserOsAlphaFeaturesName[] =
+    "BrowserOS Alpha Features";
+inline constexpr char kBrowserOsAlphaFeaturesDescription[] =
+    "Enables BrowserOS alpha features.";
+
+inline constexpr char kBrowserOsKeyboardShortcutsName[] =
+    "BrowserOS Keyboard Shortcuts";
+inline constexpr char kBrowserOsKeyboardShortcutsDescription[] =
+    "Enables BrowserOS keyboard shortcuts (Cmd+Shift+K, Cmd+Shift+L, "
+    "Option+A). Disable if these conflict with your keyboard layout.";
+
 inline constexpr char kBrowsingHistoryActorIntegrationM3Name[] =
     "Browsing History Actor Integration M3";
 inline constexpr char kBrowsingHistoryActorIntegrationM3Description[] =
@@ -463,7 +474,6 @@ inline constexpr char
         "Search. "
         "Requires #customize-chrome-wallpaper-search to be enabled too.";
 
-
 inline constexpr char kEnableCancelUploadOnContentAnalysisName[] =
     "Cancel file uploads on content analysis";
 inline constexpr char kEnableCancelUploadOnContentAnalysisDescription[] =
@@ -1390,7 +1400,6 @@ inline constexpr char kSettingsInTabName[] = "Settings in Tab";
 inline constexpr char kSettingsInTabDescription[] =
     "Allows the Chrome Settings UI to appear in a tab on Android.";
 
-
 inline constexpr char kSeparateWebAppShortcutBadgeIconName[] =
     "Separate Web App Shortcut Badge Icon";
 inline constexpr char kSeparateWebAppShortcutBadgeIconDescription[] =
@@ -3702,9 +3711,7 @@ inline constexpr char kWebUIOmniboxHideAimUrlName[] =
 inline constexpr char kWebUIOmniboxHideAimUrlDescription[] =
     "If enabled, hides the AIM button when the default suggestion is a URL.";
 
-
-inline constexpr char kWebUIOmniboxFullPopupName[] =
-    "WebUI Omnibox Full Popup";
+inline constexpr char kWebUIOmniboxFullPopupName[] = "WebUI Omnibox Full Popup";
 inline constexpr char kWebUIOmniboxFullPopupDescription[] =
     "If enabled, then both the input row and suggestions dropdown (in the "
     "Omnibox) will be rendered using the WebUI stack (i.e. the cutout for the "
@@ -4528,7 +4535,6 @@ inline constexpr char kTabSwitcherGroupSuggestionsTestModeAndroidDescription[] =
     "Helper flag for testing that shows group suggestions for the last 3 tabs "
     "in the tab switcher (if present).";
 
-
 inline constexpr char kDataSharingDebugLogsName[] =
     "Enable data sharing debug logs";
 inline constexpr char kDataSharingDebugLogsDescription[] =
@@ -6472,7 +6478,6 @@ inline constexpr char kMemoryPurgeOnFreezeLimitDescription[] =
     "backgrounded interval, to minimize overhead when pages are periodically "
     "unfrozen. To be enabled with memory-purge-on-freeze-limit.";
 
-
 inline constexpr char kReadAnythingOmniboxChipName[] =
     "Reading Mode Omnibox Chip";
 inline constexpr char kReadAnythingOmniboxChipDescription[] =
