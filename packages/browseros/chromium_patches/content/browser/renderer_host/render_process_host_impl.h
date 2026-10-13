diff --git a/content/browser/renderer_host/render_process_host_impl.h b/content/browser/renderer_host/render_process_host_impl.h
index 8809a1ad6b6e8a68f178c82416cd36b873d72668..d20e4901f3c8ba99bf6efa3982b10231437edad7 100644
--- a/content/browser/renderer_host/render_process_host_impl.h
+++ b/content/browser/renderer_host/render_process_host_impl.h
@@ -279,6 +279,14 @@ class CONTENT_EXPORT RenderProcessHostImpl
   base::CallbackListSubscription RegisterBlockStateChangedCallback(
       const BlockStateChangedCallback& cb) override;
   void Cleanup() override;
+
+#if BUILDFLAG(IS_MAC)
+  // Route the manager's one-use normal reason through virtual Cleanup, so test
+  // factories need no cast and removed spares need no later membership lookup.
+  static void CleanupForNormalSpare(
+      RenderProcessHost* host,
+      internal::NormalRendererShutdownReason reason);
+#endif
   void AddPendingView() override;
   void RemovePendingView() override;
   void AddPriorityClient(
@@ -1223,12 +1231,23 @@ class CONTENT_EXPORT RenderProcessHostImpl
   ChildProcessTerminationInfo GetChildTerminationInfo(bool already_dead);
 
   // Handle termination of our process.
-  void ProcessDied(const ChildProcessTerminationInfo& termination_info);
+  void ProcessDied(const ChildProcessTerminationInfo& termination_info
+#if BUILDFLAG(IS_MAC)
+                   ,
+                   std::optional<internal::NormalRendererShutdownReason>
+                       normal_reason = std::nullopt
+#endif
+  );
 
   // Shutdowns the child process as fast as possible. This is similar to the
   // public `FastShutdownIfPossible()` method, but doesn't perform any checks
   // before initiating fast shutdown.
-  void FastShutdown();
+  void FastShutdown(
+#if BUILDFLAG(IS_MAC)
+      std::optional<internal::NormalRendererShutdownReason> normal_reason =
+          std::nullopt
+#endif
+  );
 
   // Destroy all objects that can cause methods to be invoked on this object or
   // any other that hang off it.
