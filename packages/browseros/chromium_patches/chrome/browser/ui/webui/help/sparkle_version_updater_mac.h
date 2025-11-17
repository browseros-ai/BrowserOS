diff --git a/chrome/browser/ui/webui/help/sparkle_version_updater_mac.h b/chrome/browser/ui/webui/help/sparkle_version_updater_mac.h
new file mode 100644
index 0000000000000..bdb3d62440d0d
--- /dev/null
+++ b/chrome/browser/ui/webui/help/sparkle_version_updater_mac.h
@@ -0,0 +1,58 @@
+// Copyright 2024 BlockBrowser Browser Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_UI_WEBUI_HELP_SPARKLE_VERSION_UPDATER_MAC_H_
+#define CHROME_BROWSER_UI_WEBUI_HELP_SPARKLE_VERSION_UPDATER_MAC_H_
+
+#include "chrome/browser/ui/webui/help/version_updater.h"
+#include "chrome/browser/mac/sparkle_glue.h"
+#include "base/memory/weak_ptr.h"
+
+#if defined(__OBJC__)
+@class NSNotification;
+#else
+class NSNotification;
+#endif
+
+// Enum for Sparkle update status
+enum SparkleUpdateStatus {
+  kSparkleStatusChecking,
+  kSparkleStatusNoUpdate,
+  kSparkleStatusUpdateFound,
+  kSparkleStatusDownloading,
+  kSparkleStatusReadyToInstall,
+  kSparkleStatusError
+};
+
+// SparkleVersionUpdater is the VersionUpdater implementation for macOS
+// that uses the Sparkle framework for updates.
+class SparkleVersionUpdater : public VersionUpdater {
+ public:
+  SparkleVersionUpdater();
+  SparkleVersionUpdater(const SparkleVersionUpdater&) = delete;
+  SparkleVersionUpdater& operator=(const SparkleVersionUpdater&) = delete;
+  ~SparkleVersionUpdater() override;
+
+  // VersionUpdater implementation.
+  void CheckForUpdate(StatusCallback status_callback,
+                      PromoteCallback promote_callback) override;
+  void PromoteUpdater() override;
+
+  // Called by SparkleGlue to notify of status changes
+  void OnSparkleStatusChange(SparkleUpdateStatus status, const std::string& error_message = "");
+  
+  // Called by SparkleGlue to notify of download progress
+  void OnDownloadProgress(double progress);
+
+  // Get a weak pointer to this object
+  base::WeakPtr<SparkleVersionUpdater> GetWeakPtr();
+
+ private:
+  void UpdateStatus(SparkleUpdateStatus status, const std::string& error_message = "");
+
+  StatusCallback status_callback_;
+  base::WeakPtrFactory<SparkleVersionUpdater> weak_ptr_factory_{this};
+};
+
+#endif  // CHROME_BROWSER_UI_WEBUI_HELP_SPARKLE_VERSION_UPDATER_MAC_H_
\ No newline at end of file
