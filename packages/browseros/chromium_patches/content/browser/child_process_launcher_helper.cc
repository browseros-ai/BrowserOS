diff --git a/content/browser/child_process_launcher_helper.cc b/content/browser/child_process_launcher_helper.cc
index 34a9ba1470d362323dd70cdd2f5ebb1a3cadef3b..3df7af25321a55d53ac82ecb4fad8b3eda7ac54c 100644
--- a/content/browser/child_process_launcher_helper.cc
+++ b/content/browser/child_process_launcher_helper.cc
@@ -185,6 +185,12 @@ ChildProcessLauncherHelper::Process::~Process() = default;
 
 ChildProcessLauncherHelper::Process::Process(Process&& other)
     : process(std::move(other.process))
+#if BUILDFLAG(IS_MAC)
+      ,
+      renderer_shutdown_intent(std::move(other.renderer_shutdown_intent)),
+      renderer_shutdown_diagnostic_disqualified(
+          other.renderer_shutdown_diagnostic_disqualified)
+#endif
 #if BUILDFLAG(USE_ZYGOTE)
       ,
       zygote(other.zygote)
@@ -474,6 +480,9 @@ void ChildProcessLauncherHelper::ForceNormalProcessTerminationAsync(
   }
   // On Posix, EnsureProcessTerminated can lead to 2 seconds of sleep!
   // So don't do this on the UI/IO threads.
+  // On Mac, the Process move also transfers its immutable diagnostic intent.
+  // A launcher-thread result must correlate by that token, not by log thread
+  // ID.
   GetProcessLauncherTaskRunner()->PostTask(
       FROM_HERE,
       base::BindOnce(
