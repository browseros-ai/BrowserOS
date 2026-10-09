diff --git a/chrome/browser/chrome_browser_main_win.cc b/chrome/browser/chrome_browser_main_win.cc
index 14bf87c503bb167c79f838b82fcf4810ddede740..91c94b2bc86f7264c52806de7a010ac94150d987 100644
--- a/chrome/browser/chrome_browser_main_win.cc
+++ b/chrome/browser/chrome_browser_main_win.cc
@@ -58,6 +58,7 @@
 #include "chrome/browser/active_use_util.h"
 #include "chrome/browser/browser_features.h"
 #include "chrome/browser/browser_process.h"
+#include "chrome/browser/buildflags.h"
 #include "chrome/browser/first_run/first_run.h"
 #include "chrome/browser/first_run/upgrade_util.h"
 #include "chrome/browser/first_run/upgrade_util_win.h"
@@ -137,6 +138,10 @@
 #include "chrome/browser/platform_experience/installer/installer_win.h"
 #endif  // BUILDFLAG(GOOGLE_CHROME_BRANDING)
 
+#if BUILDFLAG(ENABLE_WINSPARKLE)
+#include "chrome/browser/win/winsparkle_glue.h"
+#endif  // BUILDFLAG(ENABLE_WINSPARKLE)
+
 namespace {
 
 typedef HRESULT (STDAPICALLTYPE* RegisterApplicationRestartProc)(
@@ -660,6 +665,11 @@ int ChromeBrowserMainPartsWin::PostCreateThreads() {
 }
 
 void ChromeBrowserMainPartsWin::PostMainMessageLoopRun() {
+#if BUILDFLAG(ENABLE_WINSPARKLE)
+  // Shut WinSparkle down while the task system is still alive.
+  winsparkle_glue::Cleanup();
+#endif
+
   base::ImportantFileWriterCleaner::GetInstance().Stop();
 
   ChromeBrowserMainParts::PostMainMessageLoopRun();
@@ -720,6 +730,12 @@ void ChromeBrowserMainPartsWin::PostBrowserStart() {
 
   InitializeChromeElf();
 
+#if BUILDFLAG(ENABLE_WINSPARKLE)
+  // Start the WinSparkle auto-updater. Must come after browser start so its
+  // first automatic check (and any update UI) runs behind a visible browser.
+  winsparkle_glue::Initialize();
+#endif
+
 #if BUILDFLAG(USE_GOOGLE_UPDATE_INTEGRATION)
   if constexpr (kShouldRecordActiveUse) {
     did_run_updater_.emplace();
