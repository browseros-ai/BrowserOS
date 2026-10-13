diff --git a/content/browser/child_process_launcher.h b/content/browser/child_process_launcher.h
index 78aa9d6bfbb55f6b66faa572ea289fefc3ead74a..66c600adf23cf052bd52e7665b9f058c51f99fa4 100644
--- a/content/browser/child_process_launcher.h
+++ b/content/browser/child_process_launcher.h
@@ -316,6 +316,15 @@ class CONTENT_EXPORT ChildProcessLauncher
   // Note that |exit_code| is not used on Android.
   bool Terminate(int exit_code);
 
+#if BUILDFLAG(IS_MAC)
+  // Diagnostic authority is supplied only by accepted RPH lifecycle decisions.
+  // Generic cleanup/termination permanently invalidates this Process's context.
+  void RecordNormalRendererShutdownIntentForDiagnostics(
+      ChildProcessId host_id,
+      internal::NormalRendererShutdownReason reason);
+  void InvalidateRendererShutdownIntentForDiagnostics();
+#endif
+
   // Similar to Terminate() but takes in a |process|.
   // On Android |process| must have been started by ChildProcessLauncher for
   // this method to work.
