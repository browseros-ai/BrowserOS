diff --git a/chrome/browser/lifetime/browser_shutdown.cc b/chrome/browser/lifetime/browser_shutdown.cc
index 70c82fe2723b8b7f2ac50a3a1bb4191d4168abe7..b0852177d94295779074ea7c687c7cfa495b932b 100644
--- a/chrome/browser/lifetime/browser_shutdown.cc
+++ b/chrome/browser/lifetime/browser_shutdown.cc
@@ -172,6 +172,22 @@ ShutdownType GetShutdownType() {
   return g_shutdown_type;
 }

+void ResetAbortedShutdown() {
+  CheckAccessedOnCorrectThread();
+  if ((g_browser_process && g_browser_process->IsShuttingDown()) ||
+      g_shutdown_type == ShutdownType::kEndSession) {
+    return;
+  }
+  g_shutdown_type = ShutdownType::kNotValid;
+  delete g_shutdown_started;
+  g_shutdown_started = nullptr;
+  if (g_browser_process) {
+    SetTryingToQuit(false);
+  } else {
+    g_trying_to_quit = false;
+  }
+}
+
 #if !BUILDFLAG(IS_ANDROID)
 RestartMode ShutdownPreThreadsStop() {
 #if BUILDFLAG(IS_CHROMEOS)
