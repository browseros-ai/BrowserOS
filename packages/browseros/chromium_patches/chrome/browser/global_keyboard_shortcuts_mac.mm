diff --git a/chrome/browser/global_keyboard_shortcuts_mac.mm b/chrome/browser/global_keyboard_shortcuts_mac.mm
index bc8c1e5d952c1..5654e5f88c6a3 100644
--- a/chrome/browser/global_keyboard_shortcuts_mac.mm
+++ b/chrome/browser/global_keyboard_shortcuts_mac.mm
@@ -15,6 +15,7 @@
 #include "chrome/app/chrome_command_ids.h"
 #import "chrome/browser/app_controller_mac.h"
 #include "chrome/browser/ui/cocoa/accelerators_cocoa.h"
+#include "chrome/browser/browser_features.h"
 #include "chrome/browser/ui/tabs/features.h"
 #include "chrome/browser/ui/ui_features.h"
 #include "ui/base/accelerators/accelerator.h"
@@ -165,6 +166,18 @@ CommandForKeyEventResult ShortcutCommand(int cmd) {
     keys.push_back(
         {true, false, true, false, kVK_ANSI_Z, IDC_FOCUS_PREV_TAB_GROUP});
 
+    if (base::FeatureList::IsEnabled(features::kBrowserOsKeyboardShortcuts)) {
+      keys.push_back(
+          {true, true, false, false, kVK_ANSI_K,
+           IDC_SHOW_THIRD_PARTY_LLM_SIDE_PANEL});
+      keys.push_back(
+          {true, true, false, false, kVK_ANSI_L,
+           IDC_CYCLE_THIRD_PARTY_LLM_PROVIDER});
+      keys.push_back(
+          {false, false, false, true, kVK_ANSI_A,
+           IDC_TOGGLE_BROWSEROS_AGENT});
+    }
+
     if (base::FeatureList::IsEnabled(features::kUIDebugTools)) {
       keys.push_back(
           {false, true, true, true, kVK_ANSI_T, IDC_DEBUG_TOGGLE_TABLET_MODE});
