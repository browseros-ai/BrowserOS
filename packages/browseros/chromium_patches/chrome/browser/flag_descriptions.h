diff --git a/chrome/browser/flag_descriptions.h b/chrome/browser/flag_descriptions.h
index 37ed204c78835763406703646ef0f97a381a136f..1be4f261cc21f09a0cacd5454b63c27915ded7f1 100644
--- a/chrome/browser/flag_descriptions.h
+++ b/chrome/browser/flag_descriptions.h
@@ -325,6 +325,18 @@ inline constexpr char kBlockingFocusWithoutUserActivationDescription[] =
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
