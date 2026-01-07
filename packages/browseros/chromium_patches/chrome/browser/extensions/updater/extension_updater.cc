diff --git a/chrome/browser/extensions/updater/extension_updater.cc b/chrome/browser/extensions/updater/extension_updater.cc
index fb3182b3a431c..d63fbc9af10c6 100644
--- a/chrome/browser/extensions/updater/extension_updater.cc
+++ b/chrome/browser/extensions/updater/extension_updater.cc
@@ -610,6 +610,86 @@ void ExtensionUpdater::CheckNow(CheckParams params) {
   }
 }
 
+void ExtensionUpdater::InstallPendingNow(CheckParams params) {
+  CHECK(enabled_);
+
+  if (params.ids.empty()) {
+    // If no IDs specified, nothing to do
+    if (params.callback) {
+      std::move(params.callback).Run();
+    }
+    return;
+  }
+
+  int request_id = next_request_id_++;
+  VLOG(2) << "Starting pending extension install " << request_id;
+
+  DCHECK(alive_);
+
+  InProgressCheck& request = requests_in_progress_[request_id];
+  request.update_found_callback = params.update_found_callback;
+  request.callback = std::move(params.callback);
+  request.install_immediately = params.install_immediately;
+  request.profile_keep_alive = std::make_unique<ScopedProfileKeepAlive>(
+      profile_, ProfileKeepAliveOrigin::kExtensionUpdater);
+
+  EnsureDownloaderCreated();
+
+  ExtensionUpdateCheckParams update_check_params;
+
+  for (const ExtensionId& id : params.ids) {
+    const PendingExtensionInfo* info = pending_extension_manager_->GetById(id);
+    if (!info) {
+      VLOG(2) << "Extension " << id << " not in pending manager, skipping";
+      continue;
+    }
+
+    if (!Manifest::IsAutoUpdateableLocation(info->install_source())) {
+      VLOG(2) << "Extension " << id << " is not auto updateable";
+      continue;
+    }
+
+    const bool is_corrupt_reinstall =
+        corrupted_extension_reinstaller_->IsReinstallForCorruptionExpected(id);
+
+    if (CanUseUpdateService(id)) {
+      update_check_params.update_info[id] = GetExtensionUpdateData(id);
+      update_check_params.update_info[id].is_corrupt_reinstall =
+          is_corrupt_reinstall;
+    } else if (downloader_->AddPendingExtension(ExtensionDownloaderTask(
+                   id, info->update_url(), info->install_source(),
+                   is_corrupt_reinstall, request_id, params.fetch_priority))) {
+      request.in_progress_ids.insert(id);
+      InstallStageTracker::Get(profile_)->ReportInstallationStage(
+          id, InstallStageTracker::Stage::DOWNLOADING);
+    } else {
+      InstallStageTracker::Get(profile_)->ReportFailure(
+          id, InstallStageTracker::FailureReason::DOWNLOADER_ADD_FAILED);
+    }
+  }
+
+  bool empty_downloader = request.in_progress_ids.empty();
+  bool awaiting_update_service = !update_check_params.update_info.empty();
+
+  request.awaiting_update_service = awaiting_update_service;
+
+  downloader_->StartAllPending(extension_cache_);
+
+  if (awaiting_update_service) {
+    update_check_params.priority =
+        params.fetch_priority == DownloadFetchPriority::kBackground
+            ? ExtensionUpdateCheckParams::UpdateCheckPriority::BACKGROUND
+            : ExtensionUpdateCheckParams::UpdateCheckPriority::FOREGROUND;
+    update_check_params.install_immediately = params.install_immediately;
+    update_service_->StartUpdateCheck(
+        update_check_params, params.update_found_callback,
+        base::BindOnce(&ExtensionUpdater::OnUpdateServiceFinished,
+                       base::Unretained(this), request_id));
+  } else if (empty_downloader) {
+    NotifyIfFinished(request_id);
+  }
+}
+
 void ExtensionUpdater::OnExtensionDownloadStageChanged(const ExtensionId& id,
                                                        Stage stage) {
   InstallStageTracker::Get(profile_)->ReportDownloadingStage(id, stage);
