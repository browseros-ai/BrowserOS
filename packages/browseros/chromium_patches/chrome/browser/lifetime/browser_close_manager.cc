diff --git a/chrome/browser/lifetime/browser_close_manager.cc b/chrome/browser/lifetime/browser_close_manager.cc
index 78aa67714c02471e7167c3d7514b357386e3e3fb..e6cc3723cf73cd879b8d5d404ca010c2ea598a4c 100644
--- a/chrome/browser/lifetime/browser_close_manager.cc
+++ b/chrome/browser/lifetime/browser_close_manager.cc
@@ -195,11 +195,9 @@ void BrowserCloseManager::CloseBrowsers() {
   ProfileManager::ShutdownSessionServices();
 #endif
 #if BUILDFLAG(ENABLE_BACKGROUND_MODE)
-  if (!browser_shutdown::IsTryingToQuit()) {
-    BackgroundModeManager* background_mode_manager =
-        g_browser_process->background_mode_manager();
-    if (background_mode_manager) {
-      background_mode_manager->SuspendBackgroundMode();
-    }
+  BackgroundModeManager* background_mode_manager =
+      g_browser_process->background_mode_manager();
+  if (background_mode_manager) {
+    background_mode_manager->SuspendBackgroundMode();
   }
 #endif
