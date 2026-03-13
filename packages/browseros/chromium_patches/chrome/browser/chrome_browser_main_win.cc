diff --git a/chrome/browser/chrome_browser_main_win.cc b/chrome/browser/chrome_browser_main_win.cc
index 6e1d22522a8f5..53f13bc73fbd1 100644
--- a/chrome/browser/chrome_browser_main_win.cc
+++ b/chrome/browser/chrome_browser_main_win.cc
@@ -51,6 +51,11 @@
 #include "base/win/wrapped_window_proc.h"
 #include "build/branding_buildflags.h"
 #include "build/build_config.h"
+#include "chrome/browser/buildflags.h"
+
+#if BUILDFLAG(ENABLE_WIN_SPARKLE)
+#include "chrome/browser/win/winsparkle_glue.h"
+#endif
 #include "chrome/browser/about_flags.h"
 #include "chrome/browser/active_use_util.h"
 #include "chrome/browser/browser_features.h"
@@ -572,6 +577,12 @@ void ChromeBrowserMainPartsWin::PreCreateMainMessageLoop() {
     // Make sure that we know how to handle exceptions from the message loop.
     InitializeWindowProcExceptions();
   }
+
+#if BUILDFLAG(ENABLE_WIN_SPARKLE)
+  // Initialize WinSparkle. This handles all setup internally, including
+  // checking if updates are disabled via command line.
+  winsparkle_glue::WinSparkleGlue::GetInstance()->Initialize();
+#endif
 }
 
 int ChromeBrowserMainPartsWin::PreCreateThreads() {
@@ -619,6 +630,10 @@ void ChromeBrowserMainPartsWin::PostMainMessageLoopRun() {
   // requests will be created.
   platform_auth_policy_observer_.reset();
 
+#if BUILDFLAG(ENABLE_WIN_SPARKLE)
+  winsparkle_glue::WinSparkleGlue::GetInstance()->Shutdown();
+#endif
+
   ChromeBrowserMainParts::PostMainMessageLoopRun();
 }
 
