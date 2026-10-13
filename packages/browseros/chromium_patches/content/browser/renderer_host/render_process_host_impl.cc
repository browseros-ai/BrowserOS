diff --git a/content/browser/renderer_host/render_process_host_impl.cc b/content/browser/renderer_host/render_process_host_impl.cc
index b8d41ae535d8a8cbfdaf3a252975d612bf7a3df4..55a852519b03c4ebb65c8843ff86509f02ae0cdf 100644
--- a/content/browser/renderer_host/render_process_host_impl.cc
+++ b/content/browser/renderer_host/render_process_host_impl.cc
@@ -18,6 +18,7 @@
 #include <vector>
 
 #include "base/atomicops.h"
+#include "base/auto_reset.h"
 #include "base/base_switches.h"
 #include "base/byte_size.h"
 #include "base/clang_profiling_buildflags.h"
@@ -308,6 +309,22 @@ namespace content {
 
 namespace {
 
+#if BUILDFLAG(IS_MAC)
+// UI-only authority borrowed across exactly one synchronous virtual Cleanup.
+// The first matching entry takes the reason even if admission later returns;
+// nested cleanup, mocks and later incarnations cannot borrow it again.
+struct NormalSpareCleanupDiagnosticScope {
+  // Only compare this borrowed pointer during the call; never
+  // retain/dereference it afterward. A test factory may delete its host in its
+  // virtual Cleanup.
+  RAW_PTR_EXCLUSION RenderProcessHost* host;
+  internal::NormalRendererShutdownReason reason;
+  bool taken = false;
+};
+NormalSpareCleanupDiagnosticScope* g_normal_spare_cleanup_diagnostic_scope =
+    nullptr;
+#endif
+
 using perfetto::protos::pbzero::ChromeTrackEvent;
 
 // Stores the maximum number of renderer processes the content module can
@@ -4309,7 +4326,11 @@ bool RenderProcessHostImpl::FastShutdownIfPossible(
     return false;
   }
 
-  FastShutdown();
+  FastShutdown(
+#if BUILDFLAG(IS_MAC)
+      internal::NormalRendererShutdownReason::kAcceptedFastShutdown
+#endif
+  );
   LogDelayReasonForFastShutdown(DelayShutdownReason::kNoDelay);
   return true;
 }
@@ -4466,7 +4487,39 @@ bool RenderProcessHostImpl::HasOnlyNonLiveRenderFrameHosts() {
   return true;
 }
 
+#if BUILDFLAG(IS_MAC)
+// static
+void RenderProcessHostImpl::CleanupForNormalSpare(
+    RenderProcessHost* host,
+    internal::NormalRendererShutdownReason reason) {
+  if (!internal::ChildProcessLauncherHelper::
+          RendererShutdownDiagnosticsEnabled() ||
+      !BrowserThread::CurrentlyOn(BrowserThread::UI) ||
+      !host->IsUnused()) {
+    host->Cleanup();
+    return;
+  }
+  NormalSpareCleanupDiagnosticScope scope{.host = host, .reason = reason};
+  base::AutoReset<NormalSpareCleanupDiagnosticScope*> reset(
+      &g_normal_spare_cleanup_diagnostic_scope, &scope);
+  // Preserve dispatch to factory-created hosts and the original cleanup order.
+  host->Cleanup();
+}
+#endif
+
 void RenderProcessHostImpl::Cleanup() {
+#if BUILDFLAG(IS_MAC)
+  std::optional<internal::NormalRendererShutdownReason> normal_spare_reason;
+  if (internal::ChildProcessLauncherHelper::
+          RendererShutdownDiagnosticsEnabled() &&
+      BrowserThread::CurrentlyOn(BrowserThread::UI)) {
+    if (auto* scope = g_normal_spare_cleanup_diagnostic_scope;
+        scope && scope->host == this && !scope->taken) {
+      scope->taken = true;
+      normal_spare_reason = scope->reason;
+    }
+  }
+#endif
   TRACE_EVENT("shutdown", "RenderProcessHostImpl::Cleanup",
               ChromeTrackEvent::kRenderProcessHost, *this);
   CHECK_CURRENTLY_ON(BrowserThread::UI, base::NotFatalUntil::M152);
@@ -4600,7 +4653,11 @@ void RenderProcessHostImpl::Cleanup() {
                 "RenderProcessHostImpl::Cleanup : Exit without full cleanup.",
                 ChromeTrackEvent::kRenderProcessHost, *this);
 
-    FastShutdown();
+    FastShutdown(
+#if BUILDFLAG(IS_MAC)
+        normal_spare_reason
+#endif
+    );
     return;
   }
 
@@ -4637,6 +4694,15 @@ void RenderProcessHostImpl::Cleanup() {
   if (IsInitializedAndNotDead()) {
     // Populates Android-only fields and closes the underlying base::Process.
     ChildProcessTerminationInfo info = GetChildTerminationInfo(false);
+#if BUILDFLAG(IS_MAC)
+    // Only this call's admitted manager reason can mint. The real status is
+    // checked before the existing observer-only NORMAL/0 substitution.
+    if (normal_spare_reason && child_process_launcher_ &&
+        info.status == base::TERMINATION_STATUS_STILL_RUNNING) {
+      child_process_launcher_->RecordNormalRendererShutdownIntentForDiagnostics(
+          GetID(), *normal_spare_reason);
+    }
+#endif
     info.status = base::TERMINATION_STATUS_NORMAL_TERMINATION;
     info.exit_code = 0;
     for (auto& observer : observers_) {
@@ -5690,7 +5756,12 @@ ChildProcessTerminationInfo RenderProcessHostImpl::GetChildTerminationInfo(
 }
 
 void RenderProcessHostImpl::ProcessDied(
-    const ChildProcessTerminationInfo& termination_info) {
+    const ChildProcessTerminationInfo& termination_info
+#if BUILDFLAG(IS_MAC)
+    ,
+    std::optional<internal::NormalRendererShutdownReason> normal_reason
+#endif
+) {
   TRACE_EVENT0("content", "RenderProcessHostImpl::ProcessDied");
   // Our child process has died.  If we didn't expect it, it's a crash.
   // In any case, we need to let everyone know it's gone.
@@ -5712,6 +5783,18 @@ void RenderProcessHostImpl::ProcessDied(
   // while we are dying.
   CHECK(!deleting_soon_, base::NotFatalUntil::M152);
 
+#if BUILDFLAG(IS_MAC)
+  if (child_process_launcher_) {
+    if (normal_reason) {
+      child_process_launcher_->RecordNormalRendererShutdownIntentForDiagnostics(
+          GetID(), *normal_reason);
+    } else {
+      // Channel errors/crashes and generic cleanup cannot inherit normal
+      // authority from the launcher's destructor or synthetic observer status.
+      child_process_launcher_->InvalidateRendererShutdownIntentForDiagnostics();
+    }
+  }
+#endif
   child_process_launcher_.reset();
   is_dead_ = true;
   // Make sure no IPCs or mojo calls from the old process get dispatched after
@@ -5764,7 +5847,11 @@ void RenderProcessHostImpl::ProcessDied(
   // TODO(darin): clean this up
 }
 
-void RenderProcessHostImpl::FastShutdown() {
+void RenderProcessHostImpl::FastShutdown(
+#if BUILDFLAG(IS_MAC)
+    std::optional<internal::NormalRendererShutdownReason> normal_reason
+#endif
+) {
   // Set this before ProcessDied() so observers can know that process died due
   // to fast shutdown.
   fast_shutdown_started_ = true;
@@ -5773,10 +5860,20 @@ void RenderProcessHostImpl::FastShutdown() {
   // destroyed a little bit later. Observers shouldn't rely on this process
   // anymore.
   auto termination_info = GetChildTerminationInfo(/* already_dead=*/false);
+#if BUILDFLAG(IS_MAC)
+  if (termination_info.status != base::TERMINATION_STATUS_STILL_RUNNING) {
+    normal_reason.reset();
+  }
+#endif
   termination_info.status = base::TERMINATION_STATUS_NORMAL_TERMINATION;
   termination_info.exit_code = 0;
 
-  ProcessDied(termination_info);
+  ProcessDied(termination_info
+#if BUILDFLAG(IS_MAC)
+              ,
+              normal_reason
+#endif
+  );
 }
 
 void RenderProcessHostImpl::ResetIPC() {
