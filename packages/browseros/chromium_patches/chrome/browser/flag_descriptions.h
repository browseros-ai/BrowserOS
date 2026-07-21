diff --git a/chrome/browser/flag_descriptions.h b/chrome/browser/flag_descriptions.h
index 17f541b40aa7cf85e95c4fff9a2d2dcd22166354..d5d596bd53ba903a8be41783988995c6b4c29608 100644
--- a/chrome/browser/flag_descriptions.h
+++ b/chrome/browser/flag_descriptions.h
@@ -314,6 +314,18 @@ inline constexpr char kBlockingFocusWithoutUserActivationDescription[] =
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
