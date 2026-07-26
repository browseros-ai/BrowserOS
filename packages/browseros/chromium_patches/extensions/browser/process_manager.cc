diff --git a/extensions/browser/process_manager.cc b/extensions/browser/process_manager.cc
index b80e711ccdeae..790a4d60b32c2 100644
--- a/extensions/browser/process_manager.cc
+++ b/extensions/browser/process_manager.cc
@@ -21,6 +21,7 @@
 #include "base/time/time.h"
 #include "base/trace_event/trace_event.h"
 #include "base/uuid.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "content/public/browser/browser_context.h"
 #include "content/public/browser/browser_task_traits.h"
 #include "content/public/browser/browser_thread.h"
@@ -80,7 +81,7 @@ ExtensionId GetExtensionID(content::RenderFrameHost* render_frame_host) {
 bool IsFrameInExtensionHost(ExtensionHost* extension_host,
                             content::RenderFrameHost* render_frame_host) {
   return content::WebContents::FromRenderFrameHost(render_frame_host) ==
-      extension_host->host_contents();
+         extension_host->host_contents();
 }
 
 // Incognito profiles use this process manager. It is mostly a shim that decides
@@ -102,8 +103,8 @@ class IncognitoProcessManager : public ProcessManager {
                             const GURL& url) override;
 };
 
-static void CreateBackgroundHostForExtensionLoad(
-    ProcessManager* manager, const Extension* extension) {
+static void CreateBackgroundHostForExtensionLoad(ProcessManager* manager,
+                                                 const Extension* extension) {
   if (BackgroundInfo::HasPersistentBackgroundPage(extension)) {
     manager->CreateBackgroundHost(extension,
                                   BackgroundInfo::GetBackgroundURL(extension));
@@ -227,8 +228,9 @@ void ProcessManager::Shutdown() {
   DCHECK(background_hosts_.empty());
   content::DevToolsAgentHost::RemoveObserver(this);
 
-  for (auto& observer : observer_list_)
+  for (auto& observer : observer_list_) {
     observer.OnProcessManagerShutdown(this);
+  }
 }
 
 void ProcessManager::RegisterRenderFrameHost(
@@ -243,8 +245,9 @@ void ProcessManager::RegisterRenderFrameHost(
   // UnregisterRenderFrame.
   AcquireLazyKeepaliveCountForFrame(render_frame_host);
 
-  for (auto& observer : observer_list_)
+  for (auto& observer : observer_list_) {
     observer.OnExtensionFrameRegistered(extension->id(), render_frame_host);
+  }
 }
 
 void ProcessManager::UnregisterRenderFrameHost(
@@ -257,15 +260,17 @@ void ProcessManager::UnregisterRenderFrameHost(
     ReleaseLazyKeepaliveCountForFrame(render_frame_host);
     all_extension_frames_.erase(frame);
 
-    for (auto& observer : observer_list_)
+    for (auto& observer : observer_list_) {
       observer.OnExtensionFrameUnregistered(extension_id, render_frame_host);
+    }
   }
 }
 
 const ProcessManager::FrameSet ProcessManager::GetAllFrames() const {
   FrameSet result;
-  for (const auto& key_value : all_extension_frames_)
+  for (const auto& key_value : all_extension_frames_) {
     result.insert(key_value.first);
+  }
   return result;
 }
 
@@ -307,9 +312,10 @@ bool ProcessManager::CreateBackgroundHost(const Extension* extension,
   // Don't create hosts if the embedder doesn't allow it.
   ProcessManagerDelegate* delegate =
       ExtensionsBrowserClient::Get()->GetProcessManagerDelegate();
-  if (delegate &&
-      !delegate->IsExtensionBackgroundPageAllowed(browser_context_, *extension))
+  if (delegate && !delegate->IsExtensionBackgroundPageAllowed(browser_context_,
+                                                              *extension)) {
     return false;
+  }
 
   // Don't create multiple background hosts for an extension.
   if (GetBackgroundHostForExtension(extension->id())) {
@@ -357,15 +363,17 @@ void ProcessManager::MaybeCreateStartupBackgroundHosts() {
   ProcessManagerDelegate* delegate =
       ExtensionsBrowserClient::Get()->GetProcessManagerDelegate();
   if (delegate &&
-      !delegate->AreBackgroundPagesAllowedForContext(browser_context_))
+      !delegate->AreBackgroundPagesAllowedForContext(browser_context_)) {
     return;
+  }
 
   // The embedder might want to defer background page loading. For example,
   // Chrome defers background page loading when it is launched to show the app
   // list, then triggers a load later when a browser window opens.
   if (delegate &&
-      delegate->DeferCreatingStartupBackgroundHosts(browser_context_))
+      delegate->DeferCreatingStartupBackgroundHosts(browser_context_)) {
     return;
+  }
 
   CreateStartupBackgroundHosts();
   startup_background_hosts_created_ = true;
@@ -494,8 +502,9 @@ void ProcessManager::DecrementLazyKeepaliveCount(
 void ProcessManager::NotifyExtensionProcessTerminated(
     const Extension* extension) {
   DCHECK(extension);
-  for (auto& observer : observer_list_)
+  for (auto& observer : observer_list_) {
     observer.OnExtensionProcessTerminated(extension);
+  }
 }
 
 ProcessManager::ActivitiesMultiset ProcessManager::GetLazyKeepaliveActivities(
@@ -537,8 +546,8 @@ void ProcessManager::OnSuspendAck(const ExtensionId& extension_id) {
 void ProcessManager::NetworkRequestStarted(
     content::RenderFrameHost* render_frame_host,
     uint64_t request_id) {
-  ExtensionHost* host = GetBackgroundHostForExtension(
-      GetExtensionID(render_frame_host));
+  ExtensionHost* host =
+      GetBackgroundHostForExtension(GetExtensionID(render_frame_host));
   if (!host || !IsFrameInExtensionHost(host, render_frame_host)) {
     return;
   }
@@ -651,10 +660,11 @@ void ProcessManager::OnExtensionUnloaded(BrowserContext* browser_context,
 void ProcessManager::CreateStartupBackgroundHosts() {
   DCHECK(!startup_background_hosts_created_);
   for (const scoped_refptr<const Extension>& extension :
-           extension_registry_->enabled_extensions()) {
+       extension_registry_->enabled_extensions()) {
     CreateBackgroundHostForExtensionLoad(this, extension.get());
-    for (auto& observer : observer_list_)
+    for (auto& observer : observer_list_) {
       observer.OnBackgroundHostStartup(extension.get());
+    }
   }
 }
 
@@ -663,8 +673,9 @@ void ProcessManager::OnBackgroundHostCreated(ExtensionHost* host) {
   background_hosts_.insert(host);
   host->AddObserver(this);
 
-  for (auto& observer : observer_list_)
+  for (auto& observer : observer_list_) {
     observer.OnBackgroundHostCreated(host);
+  }
 }
 
 void ProcessManager::CloseBackgroundHost(ExtensionHost* host) {
@@ -675,8 +686,9 @@ void ProcessManager::CloseBackgroundHost(ExtensionHost* host) {
   // |host| should deregister itself from our structures.
   CHECK(!background_hosts_.contains(host));
 
-  for (auto& observer : observer_list_)
+  for (auto& observer : observer_list_) {
     observer.OnBackgroundHostClose(extension_id);
+  }
 }
 
 void ProcessManager::AcquireLazyKeepaliveCountForFrame(
@@ -732,7 +744,6 @@ base::Uuid ProcessManager::IncrementServiceWorkerKeepaliveCount(
 
   base::Uuid request_uuid = base::Uuid::GenerateRandomV4();
 
-
   content::ServiceWorkerContext* service_worker_context =
       util::GetServiceWorkerContextForExtensionId(extension->id(),
                                                   browser_context_);
@@ -934,8 +945,9 @@ void ProcessManager::UnregisterExtension(const ExtensionId& extension_id) {
     content::RenderFrameHost* host = it->first;
     if (GetExtensionID(host) == extension_id) {
       all_extension_frames_.erase(it++);
-      for (auto& observer : observer_list_)
+      for (auto& observer : observer_list_) {
         observer.OnExtensionFrameUnregistered(extension_id, host);
+      }
     } else {
       ++it;
     }
@@ -959,6 +971,16 @@ void ProcessManager::StartTrackingServiceWorkerRunningInstance(
   all_running_extension_workers_.Add(worker_id, browser_context_);
   worker_context_ids_[worker_id] = base::Uuid::GenerateRandomV4();
 
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
@@ -971,8 +993,9 @@ void ProcessManager::StartTrackingServiceWorkerRunningInstance(
       // These will be cleaned up in RenderProcessExited().
       process_observations_.AddObservation(render_process_host);
     }
-    for (auto& observer : observer_list_)
+    for (auto& observer : observer_list_) {
       observer.OnStartedTrackingServiceWorkerInstance(worker_id);
+    }
   }
 }
 
@@ -1009,9 +1032,10 @@ void ProcessManager::RenderProcessExited(
 #if DCHECK_IS_ON()
   // Sanity check: No worker entry should exist for any |extension_id| running
   // inside the RenderProcessHost that died.
-  for (const ExtensionId& extension_id : iter->second)
+  for (const ExtensionId& extension_id : iter->second) {
     DCHECK(all_running_extension_workers_.GetAllForExtension(extension_id)
                .empty());
+  }
 #endif
   worker_process_to_extension_ids_.erase(iter);
 }
@@ -1046,10 +1070,22 @@ void ProcessManager::StopTrackingServiceWorkerRunningInstance(
     return;
   }
 
+  // BrowserOS: Clean up permanent keepalive for BrowserOS extensions.
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
     observer.OnStoppedTrackingServiceWorkerInstance(worker_id);
+  }
 }
 
 // TODO(crbug.com/40936639): Deduplicate this method with it's other overload
@@ -1147,8 +1183,9 @@ bool IncognitoProcessManager::CreateBackgroundHost(const Extension* extension,
                                                    const GURL& url) {
   if (IncognitoInfo::IsSplitMode(extension)) {
     if (ExtensionsBrowserClient::Get()->IsExtensionIncognitoEnabled(
-            extension->id(), browser_context()))
+            extension->id(), browser_context())) {
       return ProcessManager::CreateBackgroundHost(extension, url);
+    }
   } else {
     // Do nothing. If an extension is spanning, then its original-profile
     // background page is shared with incognito, so we don't create another.
