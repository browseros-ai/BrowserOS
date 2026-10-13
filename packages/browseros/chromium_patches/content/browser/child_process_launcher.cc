diff --git a/content/browser/child_process_launcher.cc b/content/browser/child_process_launcher.cc
index 3a547026a0fb539f551f9b07b9ba1ec9f97d1f57..356a437f98204af1d70ef43cfe53000c021a5f45 100644
--- a/content/browser/child_process_launcher.cc
+++ b/content/browser/child_process_launcher.cc
@@ -308,6 +308,11 @@ ChildProcessTerminationInfo ChildProcessLauncher::GetChildTerminationInfo(
   // still running), we'll need to Terminate via ProcessWatcher. So we can't
   // close the handle here.
   if (termination_info_.status != base::TERMINATION_STATUS_STILL_RUNNING) {
+#if BUILDFLAG(IS_MAC)
+    ChildProcessLauncherHelper::InvalidateRendererShutdownIntentForDiagnostics(
+        process_,
+        internal::RendererShutdownDiagnosticInvalidation::kProcessNotRunning);
+#endif
     process_.process.Close();
   }
 
@@ -315,11 +320,36 @@ ChildProcessTerminationInfo ChildProcessLauncher::GetChildTerminationInfo(
 }
 
 bool ChildProcessLauncher::Terminate(int exit_code) {
+#if BUILDFLAG(IS_MAC)
+  // An arbitrary exit-code request, including bad-message termination, cannot
+  // acquire normal authority from a later destructor's ForceNormal path.
+  ChildProcessLauncherHelper::InvalidateRendererShutdownIntentForDiagnostics(
+      process_,
+      internal::RendererShutdownDiagnosticInvalidation::kGenericTerminate);
+#endif
   return IsStarting() ? false
                       : ChildProcessLauncherHelper::TerminateProcess(
                             GetProcess(), exit_code);
 }
 
+#if BUILDFLAG(IS_MAC)
+void ChildProcessLauncher::RecordNormalRendererShutdownIntentForDiagnostics(
+    ChildProcessId host_id,
+    internal::NormalRendererShutdownReason reason) {
+  if (host_id != helper_->child_process_id()) {
+    InvalidateRendererShutdownIntentForDiagnostics();
+  }
+  ChildProcessLauncherHelper::RecordNormalRendererShutdownIntentForDiagnostics(
+      process_, host_id, reason);
+}
+
+void ChildProcessLauncher::InvalidateRendererShutdownIntentForDiagnostics() {
+  ChildProcessLauncherHelper::InvalidateRendererShutdownIntentForDiagnostics(
+      process_,
+      internal::RendererShutdownDiagnosticInvalidation::kGenericProcessDied);
+}
+#endif
+
 // static
 bool ChildProcessLauncher::TerminateProcess(const base::Process& process,
                                             int exit_code) {
