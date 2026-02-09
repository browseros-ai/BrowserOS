diff --git a/chrome/browser/flag_descriptions.h b/chrome/browser/flag_descriptions.h
index d73e9a053eb63..e22ef4d7024af 100644
--- a/chrome/browser/flag_descriptions.h
+++ b/chrome/browser/flag_descriptions.h
@@ -284,6 +284,22 @@ inline constexpr char kBookmarksTreeViewName[] =
 inline constexpr char kBookmarksTreeViewDescription[] =
     "Show the bookmarks side panel in a tree view while in compact mode.";
 
+// BrowserOS: feature flags
+inline constexpr char kBrowserOsAlphaFeaturesName[] =
+    "BrowserOS Alpha Features";
+inline constexpr char kBrowserOsAlphaFeaturesDescription[] =
+    "Enables BrowserOS alpha features.";
+
+inline constexpr char kBrowserOsClawdbotName[] = "BrowserOS Clawdbot";
+inline constexpr char kBrowserOsClawdbotDescription[] =
+    "Enables Clawdbot Browser Relay extension.";
+
+inline constexpr char kBrowserOsKeyboardShortcutsName[] =
+    "BrowserOS Keyboard Shortcuts";
+inline constexpr char kBrowserOsKeyboardShortcutsDescription[] =
+    "Enables BrowserOS keyboard shortcuts (Cmd+Shift+K, Cmd+Shift+L, "
+    "Option+A). Disable if these conflict with your keyboard layout.";
+
 inline constexpr char kBrowsingHistoryActorIntegrationM1Name[] =
     "Browsing History Actor Integration M1";
 inline constexpr char kBrowsingHistoryActorIntegrationM1Description[] =
