diff --git a/chrome/browser/ui/webui/help/sparkle_version_updater_mac.h b/chrome/browser/ui/webui/help/sparkle_version_updater_mac.h
new file mode 100644
index 0000000000000..a915a04e3aa03
--- /dev/null
+++ b/chrome/browser/ui/webui/help/sparkle_version_updater_mac.h
@@ -0,0 +1,39 @@
+// Copyright 2024 BrowserOS Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_UI_WEBUI_HELP_SPARKLE_VERSION_UPDATER_MAC_H_
+#define CHROME_BROWSER_UI_WEBUI_HELP_SPARKLE_VERSION_UPDATER_MAC_H_
+
+#include "chrome/browser/ui/webui/help/version_updater.h"
+
+#if defined(__OBJC__)
+@class SparkleVersionUpdaterBridge;
+#else
+class SparkleVersionUpdaterBridge;
+#endif
+
+// VersionUpdater implementation for macOS using Sparkle framework.
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
+  // Called by SparkleVersionUpdaterBridge.
+  void OnStatusChanged(int status);
+  void OnProgressChanged(int percentage);
+  void OnError(const std::string& message);
+
+ private:
+  StatusCallback status_callback_;
+  SparkleVersionUpdaterBridge* __strong bridge_;
+};
+
+#endif  // CHROME_BROWSER_UI_WEBUI_HELP_SPARKLE_VERSION_UPDATER_MAC_H_
