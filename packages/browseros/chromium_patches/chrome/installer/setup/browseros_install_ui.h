diff --git a/chrome/installer/setup/browseros_install_ui.h b/chrome/installer/setup/browseros_install_ui.h
new file mode 100644
index 0000000000000000000000000000000000000000..ec4ff1db633b205076e3132d690a9c4691282678
--- /dev/null
+++ b/chrome/installer/setup/browseros_install_ui.h
@@ -0,0 +1,39 @@
+// Copyright 2026 BrowserOS Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_INSTALLER_SETUP_BROWSEROS_INSTALL_UI_H_
+#define CHROME_INSTALLER_SETUP_BROWSEROS_INSTALL_UI_H_
+
+#include <memory>
+
+#include "base/threading/platform_thread.h"
+
+namespace installer {
+
+// Minimal progress UI for interactive BrowserOS first installs. The UI runs on
+// its own thread so setup's install work can continue on the caller's thread.
+class BrowserOSInstallUI {
+ public:
+  explicit BrowserOSInstallUI(bool should_show);
+  BrowserOSInstallUI(const BrowserOSInstallUI&) = delete;
+  BrowserOSInstallUI& operator=(const BrowserOSInstallUI&) = delete;
+  ~BrowserOSInstallUI();
+
+  void Show();
+  void Close();
+  void CloseAndShowFailureMessage(int install_msg_base);
+
+ private:
+  class UIThreadDelegate;
+
+  const bool should_show_;
+  std::unique_ptr<UIThreadDelegate> ui_thread_;
+  base::PlatformThreadHandle thread_handle_;
+};
+
+void ShowBrowserOSInstallFailureMessageBox(int install_msg_base);
+
+}  // namespace installer
+
+#endif  // CHROME_INSTALLER_SETUP_BROWSEROS_INSTALL_UI_H_
