diff --git a/chrome/browser/lifetime/application_lifetime.cc b/chrome/browser/lifetime/application_lifetime.cc
index 120cee0391d43f84215726f0711409d23964514c..fe5561c0f2bdecb2f0cddae2fbe73e1139d58346 100644
--- a/chrome/browser/lifetime/application_lifetime.cc
+++ b/chrome/browser/lifetime/application_lifetime.cc
@@ -13,6 +13,7 @@
 #include "build/build_config.h"
 #include "chrome/browser/browser_process.h"
 #include "chrome/browser/browser_process_platform_part.h"
+#include "chrome/browser/buildflags.h"
 #include "chrome/browser/lifetime/browser_shutdown.h"
 #include "chrome/browser/ui/profiles/profile_picker.h"
 #include "chrome/common/buildflags.h"
@@ -29,6 +30,13 @@
 #include "chrome/browser/lifetime/application_lifetime_desktop.h"
 #endif  // !BUILDFLAG(IS_ANDROID)
 
+#if BUILDFLAG(ENABLE_SPARKLE)
+namespace sparkle_glue {
+bool IsUpdateReady();
+void InstallAndRelaunch();
+}  // namespace sparkle_glue
+#endif
+
 namespace chrome {
 
 namespace {
@@ -65,6 +73,16 @@ void AttemptUserExit() {
 }
 
 void AttemptRelaunch() {
+#if BUILDFLAG(ENABLE_SPARKLE)
+  if (sparkle_glue::IsUpdateReady()) {
+    // Sparkle quits and reopens the app itself, bypassing AttemptRestart().
+    // Mark the quit as a restart, as AttemptRestart() would, so the relaunched
+    // browser restores the session even when startup is set to a new tab.
+    g_browser_process->local_state()->SetBoolean(prefs::kWasRestarted, true);
+    sparkle_glue::InstallAndRelaunch();
+    return;
+  }
+#endif
   AttemptRestart();
 }
 
