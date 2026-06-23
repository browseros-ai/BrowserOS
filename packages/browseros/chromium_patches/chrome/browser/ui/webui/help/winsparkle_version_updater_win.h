diff --git a/chrome/browser/ui/webui/help/winsparkle_version_updater_win.h b/chrome/browser/ui/webui/help/winsparkle_version_updater_win.h
new file mode 100644
index 0000000000000..66058657abe14
--- /dev/null
+++ b/chrome/browser/ui/webui/help/winsparkle_version_updater_win.h
@@ -0,0 +1,43 @@
+// Copyright 2024 BrowserOS Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_UI_WEBUI_HELP_WINSPARKLE_VERSION_UPDATER_WIN_H_
+#define CHROME_BROWSER_UI_WEBUI_HELP_WINSPARKLE_VERSION_UPDATER_WIN_H_
+
+#include "base/memory/weak_ptr.h"
+#include "chrome/browser/ui/webui/help/version_updater.h"
+#include "chrome/browser/win/winsparkle_glue.h"
+
+// VersionUpdater for Windows backed by WinSparkle instead of Google Update;
+// drives the chrome://settings/help status while WinSparkle's own dialogs
+// handle consent and download.
+class WinSparkleVersionUpdater : public VersionUpdater,
+                                 public winsparkle_glue::WinSparkleObserver {
+ public:
+  WinSparkleVersionUpdater();
+  WinSparkleVersionUpdater(const WinSparkleVersionUpdater&) = delete;
+  WinSparkleVersionUpdater& operator=(const WinSparkleVersionUpdater&) =
+      delete;
+  ~WinSparkleVersionUpdater() override;
+
+  // VersionUpdater:
+  void CheckForUpdate(StatusCallback status_callback,
+                      PromoteCallback promote_callback) override;
+
+  // winsparkle_glue::WinSparkleObserver:
+  void OnUpdateFound() override;
+  void OnNoUpdateFound() override;
+  void OnUpdateCancelled() override;
+  void OnUpdateInstalled() override;
+  void OnUpdateError() override;
+
+ private:
+  void OnPendingRestartCheck(bool is_update_pending_restart);
+  void RunCallback(Status status);
+
+  StatusCallback status_callback_;
+  base::WeakPtrFactory<WinSparkleVersionUpdater> weak_factory_{this};
+};
+
+#endif  // CHROME_BROWSER_UI_WEBUI_HELP_WINSPARKLE_VERSION_UPDATER_WIN_H_
