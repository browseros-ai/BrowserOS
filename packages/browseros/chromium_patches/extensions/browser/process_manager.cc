diff --git a/extensions/browser/process_manager.cc b/extensions/browser/process_manager.cc
index 426d1f04cddcc..4463d95410b8e 100644
--- a/extensions/browser/process_manager.cc
+++ b/extensions/browser/process_manager.cc
@@ -36,6 +36,7 @@
 #include "content/public/browser/site_instance.h"
 #include "content/public/browser/web_contents.h"
 #include "content/public/common/url_constants.h"
+#include "chrome/browser/extensions/blockbrowser_extension_constants.h"
 #include "extensions/browser/extension_host.h"
 #include "extensions/browser/extension_registry.h"
 #include "extensions/browser/extension_system.h"
@@ -990,6 +991,19 @@ void ProcessManager::StartTrackingServiceWorkerRunningInstance(
   all_running_extension_workers_.Add(worker_id, browser_context_);
   worker_context_ids_[worker_id] = base::Uuid::GenerateRandomV4();
 
+  // BlockBrowser: Add permanent keepalive for BlockBrowser extensions to prevent
+  // their service workers from being terminated due to inactivity.
+  if (browseros::IsBlockBrowserExtension(worker_id.extension_id)) {
+    base::Uuid keepalive_uuid = IncrementServiceWorkerKeepaliveCount(
+        worker_id,
+        content::ServiceWorkerExternalRequestTimeoutType::kDoesNotTimeout,
+        Activity::PROCESS_MANAGER,
+        "blockbrowser_permanent_keepalive");
+    blockbrowser_permanent_keepalives_[worker_id] = keepalive_uuid;
+    VLOG(1) << "browseros: Added permanent keepalive for extension "
+            << worker_id.extension_id;
+  }
+
   // Observe the RenderProcessHost for cleaning up on process shutdown.
   int render_process_id = worker_id.render_process_id;
   bool inserted = worker_process_to_extension_ids_[render_process_id]
@@ -1076,6 +1090,17 @@ void ProcessManager::StopTrackingServiceWorkerRunningInstance(
     return;
   }
 
+  // BlockBrowser: Clean up permanent keepalive for BlockBrowser extensions.
+  auto keepalive_iter = blockbrowser_permanent_keepalives_.find(worker_id);
+  if (keepalive_iter != blockbrowser_permanent_keepalives_.end()) {
+    DecrementServiceWorkerKeepaliveCount(
+        worker_id, keepalive_iter->second, Activity::PROCESS_MANAGER,
+        "blockbrowser_permanent_keepalive");
+    blockbrowser_permanent_keepalives_.erase(keepalive_iter);
+    VLOG(1) << "browseros: Removed permanent keepalive for extension "
+            << worker_id.extension_id;
+  }
+
   all_running_extension_workers_.Remove(worker_id);
   worker_context_ids_.erase(worker_id);
   for (auto& observer : observer_list_)
