diff --git a/chrome/browser/browseros/server/server_updater.h b/chrome/browser/browseros/server/server_updater.h
new file mode 100644
index 0000000000000..e1d0cfbd93d28
--- /dev/null
+++ b/chrome/browser/browseros/server/server_updater.h
@@ -0,0 +1,41 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_SERVER_UPDATER_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_SERVER_UPDATER_H_
+
+#include "base/files/file_path.h"
+
+namespace browseros {
+
+// Interface for OTA update operations.
+// Abstracts the manager's interaction with the updater to enable testing.
+class ServerUpdater {
+ public:
+  virtual ~ServerUpdater() = default;
+
+  // Lifecycle management
+  virtual void Start() = 0;
+  virtual void Stop() = 0;
+
+  // Returns true if currently checking or downloading an update.
+  virtual bool IsUpdateInProgress() const = 0;
+
+  // Binary path resolution.
+  // Returns the best available server binary path - prefers downloaded
+  // version if valid and newer, falls back to bundled.
+  virtual base::FilePath GetBestServerBinaryPath() = 0;
+
+  // Resources path resolution.
+  // Returns the resources path for the best available binary.
+  virtual base::FilePath GetBestServerResourcesPath() = 0;
+
+  // Called when downloaded version is unusable (missing or crashes repeatedly).
+  // Nukes all downloaded versions, forcing fallback to bundled binary.
+  virtual void InvalidateDownloadedVersion() = 0;
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_SERVER_UPDATER_H_
