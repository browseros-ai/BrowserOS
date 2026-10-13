diff --git a/content/browser/renderer_host/spare_render_process_host_manager_impl.cc b/content/browser/renderer_host/spare_render_process_host_manager_impl.cc
index 6846c1130ea337773534721ab56cb2119c4c7fbb..83df4cc12321116bead5b76e7e5309353a08bc2b 100644
--- a/content/browser/renderer_host/spare_render_process_host_manager_impl.cc
+++ b/content/browser/renderer_host/spare_render_process_host_manager_impl.cc
@@ -854,7 +854,42 @@ void SpareRenderProcessHostManagerImpl::DestroySpare(
 
   // Make sure the RenderProcessHost object gets destroyed.
   if (!spare_rph->AreRefCountsDisabled()) {
+#if BUILDFLAG(IS_MAC)
+    using Reason = internal::NormalRendererShutdownReason;
+    std::optional<Reason> normal_reason;
+    if (dispatch_result) {
+      switch (*dispatch_result) {
+        case SpareRendererDispatchResult::kOverridden:
+          normal_reason = Reason::kUnusedSpareOverridden;
+          break;
+        case SpareRendererDispatchResult::kDestroyedNotEnabled:
+          normal_reason = Reason::kUnusedSpareNotEnabled;
+          break;
+        case SpareRendererDispatchResult::kDestroyedProcessLimit:
+          normal_reason = Reason::kUnusedSpareProcessLimit;
+          break;
+        case SpareRendererDispatchResult::kTimeout:
+          normal_reason = Reason::kUnusedSpareTimeout;
+          break;
+        case SpareRendererDispatchResult::kMemoryPressure:
+          normal_reason = Reason::kUnusedSpareMemoryPressure;
+          break;
+        default:
+          break;
+      }
+    }
+    // CleanupSpares/TrimSpares already removed the entry. Carry authority from
+    // this manager-owned removal, not a now-false IsSpare lookup. The wrapper
+    // calls virtual Cleanup and therefore never casts a test factory's host.
+    if (normal_reason && internal::ChildProcessLauncherHelper::
+                             RendererShutdownDiagnosticsEnabled()) {
+      RenderProcessHostImpl::CleanupForNormalSpare(spare_rph, *normal_reason);
+    } else {
+      spare_rph->Cleanup();
+    }
+#else
     spare_rph->Cleanup();
+#endif
   }
 
   for (auto& observer : observer_list_) {
