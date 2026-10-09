diff --git a/extensions/browser/process_manager.cc b/extensions/browser/process_manager.cc
index d097a45600c4b40ea984d6935c49f5c2b8cbbd6f..96ccf9172e2f9f23e8fb9e88f3930b718ff54ed8 100644
--- a/extensions/browser/process_manager.cc
+++ b/extensions/browser/process_manager.cc
@@ -23,6 +23,7 @@
 #include "base/time/time.h"
 #include "base/trace_event/trace_event.h"
 #include "base/uuid.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "components/back_forward_cache/back_forward_cache_disable.h"
 #include "components/back_forward_cache/disabled_reason_id.h"
 #include "content/public/browser/browser_context.h"
@@ -994,6 +995,19 @@ void ProcessManager::StartTrackingServiceWorkerRunningInstance(
   all_running_extension_workers_.Add(worker_id, browser_context_);
   worker_context_ids_[worker_id] = base::Uuid::GenerateRandomV4();
 
+  // Product extension workers provide browser services even without an open
+  // extension page. Keep each running instance alive until tracking stops;
+  // the matching UUID must be released before its worker identity is removed.
+  if (browseros::IsActiveBrowserOSExtension(worker_id.extension_id)) {
+    base::Uuid keepalive_uuid = IncrementServiceWorkerKeepaliveCount(
+        worker_id,
+        content::ServiceWorkerExternalRequestTimeoutType::kDoesNotTimeout,
+        Activity::PROCESS_MANAGER, "browseros_permanent_keepalive");
+    browseros_permanent_keepalives_[worker_id] = keepalive_uuid;
+    VLOG(1) << "browseros: Added permanent keepalive for extension "
+            << worker_id.extension_id;
+  }
+
   // Observe the RenderProcessHost for cleaning up on process shutdown.
   bool inserted = worker_process_to_extension_ids_[worker_id.render_process_id]
                       .insert(worker_id.extension_id)
@@ -1081,11 +1095,23 @@ void ProcessManager::StopTrackingServiceWorkerRunningInstance(
     return;
   }
 
+  // Balance the BrowserOS lifetime request while the worker is still tracked.
+  auto keepalive_iter = browseros_permanent_keepalives_.find(worker_id);
+  if (keepalive_iter != browseros_permanent_keepalives_.end()) {
+    DecrementServiceWorkerKeepaliveCount(worker_id, keepalive_iter->second,
+                                         Activity::PROCESS_MANAGER,
+                                         "browseros_permanent_keepalive");
+    browseros_permanent_keepalives_.erase(keepalive_iter);
+    VLOG(1) << "browseros: Removed permanent keepalive for extension "
+            << worker_id.extension_id;
+  }
+
   all_running_extension_workers_.Remove(worker_id);
   worker_context_ids_.erase(worker_id);
-  for (auto& observer : observer_list_)
+  for (auto& observer : observer_list_) {
     observer.OnStoppedTrackingServiceWorkerInstance(*browser_context(),
                                                     worker_id);
+  }
 }
 
 // TODO(crbug.com/40936639): Deduplicate this method with it's other overload
