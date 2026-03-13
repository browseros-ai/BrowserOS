diff --git a/chrome/browser/ui/webui/help/winsparkle_version_updater_win.cc b/chrome/browser/ui/webui/help/winsparkle_version_updater_win.cc
new file mode 100644
index 0000000000000..c042edc9536fb
--- /dev/null
+++ b/chrome/browser/ui/webui/help/winsparkle_version_updater_win.cc
@@ -0,0 +1,108 @@
+// Copyright 2024 BrowserOS Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/ui/webui/help/winsparkle_version_updater_win.h"
+
+#include <memory>
+#include <string>
+
+#include "base/logging.h"
+#include "base/memory/ptr_util.h"
+#include "chrome/browser/upgrade_detector/upgrade_detector.h"
+#include "chrome/browser/win/winsparkle_glue.h"
+
+WinSparkleVersionUpdater::WinSparkleVersionUpdater() = default;
+
+WinSparkleVersionUpdater::~WinSparkleVersionUpdater() {
+  winsparkle_glue::WinSparkleGlue::GetInstance()->RemoveObserver(this);
+}
+
+void WinSparkleVersionUpdater::CheckForUpdate(
+    StatusCallback status_callback,
+    PromoteCallback promote_callback) {
+  status_callback_ = std::move(status_callback);
+
+  auto* glue = winsparkle_glue::WinSparkleGlue::GetInstance();
+  if (!glue->IsInitialized()) {
+    LOG(ERROR) << "WinSparkleVersionUpdater: WinSparkle not available";
+    if (!status_callback_.is_null()) {
+      status_callback_.Run(FAILED, 0, false, false, std::string(), 0,
+                           u"WinSparkle updater not available");
+    }
+    return;
+  }
+
+  // Register as observer lazily (matches macOS Sparkle pattern).
+  glue->AddObserver(this);
+  glue->CheckForUpdates();
+}
+
+void WinSparkleVersionUpdater::OnWinSparkleStatusChanged(
+    winsparkle_glue::WinSparkleStatus status) {
+  if (status_callback_.is_null()) {
+    return;
+  }
+
+  Status update_status = CHECKING;
+  std::u16string message;
+
+  switch (status) {
+    case winsparkle_glue::WinSparkleStatus::kIdle:
+      return;
+
+    case winsparkle_glue::WinSparkleStatus::kChecking:
+      update_status = CHECKING;
+      break;
+
+    case winsparkle_glue::WinSparkleStatus::kUpdateFound:
+      // WinSparkle handles its own download/install UI, so we report
+      // NEARLY_UPDATED to indicate an update is available/in progress.
+      update_status = NEARLY_UPDATED;
+      break;
+
+    case winsparkle_glue::WinSparkleStatus::kUpToDate:
+      update_status = UPDATED;
+      break;
+
+    case winsparkle_glue::WinSparkleStatus::kError:
+      update_status = FAILED;
+      message = u"Update check failed";
+      break;
+  }
+
+  status_callback_.Run(update_status, 0, false, false, std::string(), 0,
+                       message);
+}
+
+// Factory method - provides the VersionUpdater for Windows when WinSparkle
+// is enabled.
+std::unique_ptr<VersionUpdater> VersionUpdater::Create(
+    content::WebContents* /* web_contents */) {
+  if (winsparkle_glue::WinSparkleEnabled()) {
+    LOG(INFO) << "VersionUpdater: Using WinSparkle updater";
+    return std::make_unique<WinSparkleVersionUpdater>();
+  }
+
+  // Fallback: report current state from upgrade detector.
+  LOG(INFO) << "VersionUpdater: WinSparkle not available, using basic updater";
+  const VersionUpdater::Status status =
+      UpgradeDetector::GetInstance()->is_upgrade_available()
+          ? VersionUpdater::NEARLY_UPDATED
+          : VersionUpdater::DISABLED;
+
+  // Create a minimal updater that reports the current status.
+  class BasicVersionUpdater : public VersionUpdater {
+   public:
+    explicit BasicVersionUpdater(Status initial_status)
+        : status_(initial_status) {}
+    void CheckForUpdate(StatusCallback callback, PromoteCallback) override {
+      callback.Run(status_, 0, false, false, std::string(), 0,
+                   std::u16string());
+    }
+
+   private:
+    Status status_;
+  };
+  return std::make_unique<BasicVersionUpdater>(status);
+}
