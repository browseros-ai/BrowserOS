diff --git a/chrome/browser/extensions/updater/extension_updater.cc b/chrome/browser/extensions/updater/extension_updater.cc
index bd09a39459441..8860719396ce8 100644
--- a/chrome/browser/extensions/updater/extension_updater.cc
+++ b/chrome/browser/extensions/updater/extension_updater.cc
@@ -562,6 +562,100 @@ void ExtensionUpdater::CheckNow(CheckParams params) {
   }
 }
 
+void ExtensionUpdater::InstallPendingNow(CheckParams params) {
+  std::unique_ptr<ScopedProfileKeepAlive> keep_alive =
+      ScopedProfileKeepAlive::TryAcquire(
+          profile_, ProfileKeepAliveOrigin::kExtensionUpdater);
+  if (!keep_alive) {
+    if (params.callback) {
+      std::move(params.callback).Run();
+    }
+    return;
+  }
+
+  CHECK(enabled_);
+  CHECK(alive_);
+  CHECK(pending_extension_manager_);
+
+  if (params.ids.empty()) {
+    if (params.callback) {
+      std::move(params.callback).Run();
+    }
+    return;
+  }
+
+  int request_id = next_request_id_++;
+  VLOG(2) << "Starting pending extension install " << request_id;
+
+  EnsureDownloaderCreated();
+
+  ExtensionUpdateCheckParams update_check_params;
+  update_check_params.priority =
+      params.fetch_priority == DownloadFetchPriority::kBackground
+          ? ExtensionUpdateCheckParams::UpdateCheckPriority::BACKGROUND
+          : ExtensionUpdateCheckParams::UpdateCheckPriority::FOREGROUND;
+  update_check_params.install_immediately = params.install_immediately;
+
+  InProgressCheck& request = requests_in_progress_[request_id];
+  request.update_found_callback = params.update_found_callback;
+  request.callback = std::move(params.callback);
+  request.install_immediately = params.install_immediately;
+  request.profile_keep_alive = std::move(keep_alive);
+
+  for (const ExtensionId& id : params.ids) {
+    const PendingExtensionInfo* pending_info =
+        pending_extension_manager_->GetById(id);
+    if (!pending_info) {
+      VLOG(2) << "Extension " << id << " not in pending manager, skipping";
+      continue;
+    }
+
+    if (!Manifest::IsAutoUpdateableLocation(pending_info->install_source())) {
+      VLOG(2) << "Extension " << id << " is not auto updateable";
+      continue;
+    }
+
+    const Extension* extension = registry_->GetInstalledExtension(id);
+    const bool is_corrupt_reinstall =
+        corrupted_extension_reinstaller_->IsReinstallForCorruptionExpected(id);
+
+    if (CanUseUpdateService(extension, pending_info)) {
+      update_check_params.update_info[id] = GetExtensionUpdateData(id);
+      update_check_params.update_info[id].is_corrupt_reinstall =
+          is_corrupt_reinstall;
+    } else if (downloader_->AddPendingExtension(ToDownloaderTask(
+                   id, *pending_info, request_id, params.fetch_priority,
+                   is_corrupt_reinstall))) {
+      request.in_progress_ids.insert(id);
+      InstallStageTrackerFactory::GetForBrowserContext(profile_)
+          ->ReportInstallationStage(id,
+                                    InstallStageTracker::Stage::DOWNLOADING);
+    } else {
+      InstallStageTrackerFactory::GetForBrowserContext(profile_)->ReportFailure(
+          id, InstallStageTracker::FailureReason::DOWNLOADER_ADD_FAILED);
+    }
+  }
+
+  bool awaiting_downloader = !request.in_progress_ids.empty();
+  bool awaiting_update_service = !update_check_params.update_info.empty();
+
+  if (!awaiting_downloader && !awaiting_update_service) {
+    NotifyIfFinished(request_id);
+    return;
+  }
+
+  request.awaiting_update_service = awaiting_update_service;
+
+  downloader_->StartAllPending(extension_cache_);
+
+  if (awaiting_update_service) {
+    update_service_->StartUpdateCheck(
+        update_check_params, params.update_found_callback,
+        base::BindOnce(&ExtensionUpdater::OnUpdateServiceFinished,
+                       weak_ptr_factory_.GetWeakPtr(), request_id));
+  }
+}
+
 // Only used for ExtensionDownloader callbacks.
 void ExtensionUpdater::OnExtensionDownloadStageChanged(const ExtensionId& id,
                                                        Stage stage) {
