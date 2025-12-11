diff --git a/chrome/browser/ui/views/frame/browser_window_property_manager_win.cc b/chrome/browser/ui/views/frame/browser_window_property_manager_win.cc
index 1a62480aee22c..c4d253c027d54 100644
--- a/chrome/browser/ui/views/frame/browser_window_property_manager_win.cc
+++ b/chrome/browser/ui/views/frame/browser_window_property_manager_win.cc
@@ -5,7 +5,9 @@
 #include "chrome/browser/ui/views/frame/browser_window_property_manager_win.h"
 
 #include "base/command_line.h"
+#include "base/files/file_util.h"
 #include "base/functional/bind.h"
+#include "base/path_service.h"
 #include "base/strings/utf_string_conversions.h"
 #include "base/win/windows_version.h"
 #include "chrome/browser/browser_process.h"
@@ -17,6 +19,7 @@
 #include "chrome/browser/web_applications/extensions/web_app_extension_shortcut.h"
 #include "chrome/browser/web_applications/web_app_helpers.h"
 #include "chrome/common/pref_names.h"
+#include "chrome/installer/util/install_util.h"
 #include "components/prefs/pref_service.h"
 #include "extensions/browser/extension_registry.h"
 #include "ui/base/win/shell.h"
@@ -79,6 +82,8 @@ void BrowserWindowPropertyManager::UpdateWindowProperties() {
   base::FilePath icon_path;
   std::wstring command_line_string;
   std::wstring pinned_name;
+  bool use_fallback = false;
+
   if ((browser->is_type_normal() || browser->is_type_popup()) &&
       shortcut_manager &&
       profile->GetPrefs()->HasPrefPath(prefs::kProfileIconVersion)) {
@@ -87,6 +92,18 @@ void BrowserWindowPropertyManager::UpdateWindowProperties() {
     shortcut_manager->GetShortcutProperties(profile->GetPath(), &command_line,
                                             &pinned_name, &icon_path);
     command_line_string = command_line.GetCommandLineString();
+    // Only use profile icon if it actually exists.
+    use_fallback = !base::PathExists(icon_path);
+  }
+
+  if (use_fallback) {
+    // Fallback: Set basic relaunch details using the current executable.
+    // This ensures taskbar pinning works correctly even when the profile
+    // icon hasn't been created yet or doesn't exist.
+    base::FilePath exe_path;
+    if (base::PathService::Get(base::FILE_EXE, &exe_path)) {
+      icon_path = exe_path;
+    }
   }
   ui::win::SetAppDetailsForWindow(app_id, icon_path, 0, command_line_string,
                                   pinned_name, hwnd_);
