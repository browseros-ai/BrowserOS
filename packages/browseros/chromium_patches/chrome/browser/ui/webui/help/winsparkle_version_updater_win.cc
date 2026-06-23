diff --git a/chrome/browser/ui/webui/help/winsparkle_version_updater_win.cc b/chrome/browser/ui/webui/help/winsparkle_version_updater_win.cc
new file mode 100644
index 0000000000000..85d25fbec8532
--- /dev/null
+++ b/chrome/browser/ui/webui/help/winsparkle_version_updater_win.cc
@@ -0,0 +1,73 @@
+// Copyright 2024 BrowserOS Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/ui/webui/help/winsparkle_version_updater_win.h"
+
+#include <string>
+
+#include "base/functional/bind.h"
+#include "base/task/task_traits.h"
+#include "base/task/thread_pool.h"
+#include "chrome/browser/first_run/upgrade_util.h"
+
+WinSparkleVersionUpdater::WinSparkleVersionUpdater() {
+  winsparkle_glue::AddObserver(this);
+}
+
+WinSparkleVersionUpdater::~WinSparkleVersionUpdater() {
+  winsparkle_glue::RemoveObserver(this);
+}
+
+void WinSparkleVersionUpdater::CheckForUpdate(StatusCallback status_callback,
+                                              PromoteCallback) {
+  status_callback_ = std::move(status_callback);
+  RunCallback(CHECKING);
+
+  // A background update may already be installed and only waiting for a
+  // relaunch — same short-circuit as the Google Update implementation.
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_VISIBLE},
+      base::BindOnce(&upgrade_util::IsUpdatePendingRestart),
+      base::BindOnce(&WinSparkleVersionUpdater::OnPendingRestartCheck,
+                     weak_factory_.GetWeakPtr()));
+}
+
+void WinSparkleVersionUpdater::OnPendingRestartCheck(
+    bool is_update_pending_restart) {
+  if (is_update_pending_restart) {
+    RunCallback(NEARLY_UPDATED);
+    return;
+  }
+  winsparkle_glue::CheckForUpdatesWithUI();
+}
+
+void WinSparkleVersionUpdater::OnUpdateFound() {
+  // WinSparkle's dialog is downloading / awaiting user choice; the install
+  // completion arrives via OnUpdateInstalled.
+  RunCallback(UPDATING);
+}
+
+void WinSparkleVersionUpdater::OnNoUpdateFound() {
+  RunCallback(UPDATED);
+}
+
+void WinSparkleVersionUpdater::OnUpdateCancelled() {
+  RunCallback(UPDATED);
+}
+
+void WinSparkleVersionUpdater::OnUpdateInstalled() {
+  RunCallback(NEARLY_UPDATED);
+}
+
+void WinSparkleVersionUpdater::OnUpdateError() {
+  RunCallback(FAILED);
+}
+
+void WinSparkleVersionUpdater::RunCallback(Status status) {
+  if (status_callback_.is_null()) {
+    return;
+  }
+  status_callback_.Run(status, 0, false, false, std::string(), 0,
+                       std::u16string());
+}
