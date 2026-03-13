diff --git a/chrome/browser/ui/webui/help/winsparkle_version_updater_win.h b/chrome/browser/ui/webui/help/winsparkle_version_updater_win.h
new file mode 100644
index 0000000000000..674190b3656ba
--- /dev/null
+++ b/chrome/browser/ui/webui/help/winsparkle_version_updater_win.h
@@ -0,0 +1,32 @@
+// Copyright 2024 BrowserOS Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_UI_WEBUI_HELP_WINSPARKLE_VERSION_UPDATER_WIN_H_
+#define CHROME_BROWSER_UI_WEBUI_HELP_WINSPARKLE_VERSION_UPDATER_WIN_H_
+
+#include "chrome/browser/ui/webui/help/version_updater.h"
+#include "chrome/browser/win/winsparkle_glue.h"
+
+// VersionUpdater implementation for Windows using WinSparkle.
+class WinSparkleVersionUpdater : public VersionUpdater,
+                                 public winsparkle_glue::WinSparkleObserver {
+ public:
+  WinSparkleVersionUpdater();
+  WinSparkleVersionUpdater(const WinSparkleVersionUpdater&) = delete;
+  WinSparkleVersionUpdater& operator=(const WinSparkleVersionUpdater&) = delete;
+  ~WinSparkleVersionUpdater() override;
+
+  // VersionUpdater implementation.
+  void CheckForUpdate(StatusCallback status_callback,
+                      PromoteCallback promote_callback) override;
+
+  // WinSparkleObserver implementation.
+  void OnWinSparkleStatusChanged(
+      winsparkle_glue::WinSparkleStatus status) override;
+
+ private:
+  StatusCallback status_callback_;
+};
+
+#endif  // CHROME_BROWSER_UI_WEBUI_HELP_WINSPARKLE_VERSION_UPDATER_WIN_H_
