diff --git a/chrome/browser/ui/webui/help/sparkle_version_updater_mac.mm b/chrome/browser/ui/webui/help/sparkle_version_updater_mac.mm
new file mode 100644
index 0000000000000..889b72abb254a
--- /dev/null
+++ b/chrome/browser/ui/webui/help/sparkle_version_updater_mac.mm
@@ -0,0 +1,109 @@
+// Copyright 2024 BlockBrowser Browser Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/ui/webui/help/sparkle_version_updater_mac.h"
+
+#include "base/logging.h"
+#include "base/strings/utf_string_conversions.h"
+#include "chrome/browser/mac/sparkle_glue.h"
+#include "chrome/grit/generated_resources.h"
+#include "ui/base/l10n/l10n_util.h"
+
+SparkleVersionUpdater::SparkleVersionUpdater() = default;
+SparkleVersionUpdater::~SparkleVersionUpdater() = default;
+
+void SparkleVersionUpdater::CheckForUpdate(StatusCallback status_callback,
+                                          PromoteCallback promote_callback) {
+  LOG(INFO) << "SparkleVersionUpdater: CheckForUpdate called";
+  status_callback_ = std::move(status_callback);
+
+  SparkleGlue* sparkle = [SparkleGlue sharedSparkleGlue];
+  if (!sparkle || ![sparkle isUpdateCheckEnabled]) {
+    LOG(ERROR) << "SparkleVersionUpdater: Sparkle updater not available or disabled";
+    UpdateStatus(kSparkleStatusError, "Sparkle updater not available");
+    return;
+  }
+
+  LOG(INFO) << "SparkleVersionUpdater: Starting update check";
+  // Start checking for updates
+  UpdateStatus(kSparkleStatusChecking);
+
+  // Set this updater as the current one so SparkleGlue can notify us
+  [sparkle setVersionUpdater:GetWeakPtr()];
+
+  [sparkle checkForUpdates];
+}
+
+void SparkleVersionUpdater::PromoteUpdater() {
+  // Sparkle doesn't require promotion like Google's updater
+  // This is a no-op for Sparkle
+}
+
+void SparkleVersionUpdater::OnSparkleStatusChange(SparkleUpdateStatus status, const std::string& error_message) {
+  UpdateStatus(status, error_message);
+}
+
+void SparkleVersionUpdater::OnDownloadProgress(double progress) {
+  if (status_callback_.is_null()) {
+    return;
+  }
+  
+  // Convert progress (0.0-1.0) to percentage (0-100)
+  int percentage = static_cast<int>(progress * 100);
+  
+  VLOG(1) << "Sparkle: Download progress " << percentage << "%";
+  
+  // Create a progress message
+  std::u16string progress_message = base::UTF8ToUTF16("Downloading update: " + std::to_string(percentage) + "%");
+  
+  // Update status with download progress
+  // The status callback parameters are:
+  // (Status, progress, rollback, powerwash, version, update_size, message)
+  status_callback_.Run(UPDATING, percentage, false, false, std::string(), 0,
+                       progress_message);
+}
+
+base::WeakPtr<SparkleVersionUpdater> SparkleVersionUpdater::GetWeakPtr() {
+  return weak_ptr_factory_.GetWeakPtr();
+}
+
+void SparkleVersionUpdater::UpdateStatus(SparkleUpdateStatus status, const std::string& error_message) {
+  if (status_callback_.is_null()) {
+    return;
+  }
+
+  Status update_status = CHECKING;
+  std::u16string message;
+
+  switch (status) {
+    case kSparkleStatusChecking:
+      LOG(INFO) << "SparkleVersionUpdater: Status = Checking for updates";
+      update_status = CHECKING;
+      break;
+    case kSparkleStatusNoUpdate:
+      LOG(INFO) << "SparkleVersionUpdater: Status = No update available";
+      update_status = UPDATED;
+      break;
+    case kSparkleStatusUpdateFound:
+      LOG(INFO) << "SparkleVersionUpdater: Status = Update found";
+      update_status = UPDATING;
+      break;
+    case kSparkleStatusDownloading:
+      LOG(INFO) << "SparkleVersionUpdater: Status = Downloading update";
+      update_status = UPDATING;
+      break;
+    case kSparkleStatusReadyToInstall:
+      LOG(INFO) << "SparkleVersionUpdater: Status = Ready to install update";
+      update_status = NEARLY_UPDATED;
+      break;
+    case kSparkleStatusError:
+      LOG(ERROR) << "SparkleVersionUpdater: Status = Error: " << error_message;
+      update_status = FAILED;
+      message = base::UTF8ToUTF16(error_message);
+      break;
+  }
+
+  status_callback_.Run(update_status, 0, false, false, std::string(), 0,
+                       message);
+}
\ No newline at end of file
