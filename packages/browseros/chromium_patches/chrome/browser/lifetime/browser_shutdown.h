diff --git a/chrome/browser/lifetime/browser_shutdown.h b/chrome/browser/lifetime/browser_shutdown.h
index d0f64cba09e1d317e63c8c2329305e4ed62393ed..2a546f38800ed90b79a6f2484722bde0be881c22 100644
--- a/chrome/browser/lifetime/browser_shutdown.h
+++ b/chrome/browser/lifetime/browser_shutdown.h
@@ -81,6 +81,11 @@ bool ShouldIgnoreUnloadHandlers();
 // Get the current shutdown type.
 ShutdownType GetShutdownType();

+// Clears shutdown state when all browser windows closed (or an exit attempt did
+// not unpin the process) while a non-browser ScopedKeepAlive kept the browser
+// process alive, allowing subsequent ProcessSingleton launches to open windows.
+void ResetAbortedShutdown();
+
 #if !BUILDFLAG(IS_ANDROID)
 // Performs the shutdown tasks that need to be done before
 // BrowserProcess and the various threads go away.
