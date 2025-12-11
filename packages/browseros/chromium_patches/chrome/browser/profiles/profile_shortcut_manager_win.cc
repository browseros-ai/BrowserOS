diff --git a/chrome/browser/profiles/profile_shortcut_manager_win.cc b/chrome/browser/profiles/profile_shortcut_manager_win.cc
index dccd337174469..b5756d2f14f6f 100644
--- a/chrome/browser/profiles/profile_shortcut_manager_win.cc
+++ b/chrome/browser/profiles/profile_shortcut_manager_win.cc
@@ -856,6 +856,7 @@ void ProfileShortcutManager::DisableForUnitTests() {
 bool ProfileShortcutManager::IsFeatureEnabled() {
   if (disabled_for_unit_tests)
     return false;
+  return true;
 
   base::CommandLine* command_line = base::CommandLine::ForCurrentProcess();
   if (command_line->HasSwitch(switches::kEnableProfileShortcutManager))
