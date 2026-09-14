diff --git a/chrome/browser/browseros/server/server_updater.h b/chrome/browser/browseros/server/server_updater.h
new file mode 100644
index 0000000000000000000000000000000000000000..ce435712133908e4bc75a2ea282c221559bb1767
--- /dev/null
+++ b/chrome/browser/browseros/server/server_updater.h
@@ -0,0 +1,46 @@
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
+  // Health-confirmed activation, distinct from download completion. Allows
+  // cleanup after handoff; download-only implementations need no action.
+  virtual void OnServerActivated() {}
+
+  // Compatibility path accessors. The production manager resolves one complete
+  // installation in ServerVersionStore before an updater even exists.
+  // Returns the best available server binary path - prefers downloaded
+  // version if valid and newer, falls back to bundled.
+  virtual base::FilePath GetBestServerBinaryPath() = 0;
+
+  // Resources path resolution.
+  // Returns the resources path for the best available binary.
+  virtual base::FilePath GetBestServerResourcesPath() = 0;
+
+  // Requests rejection of a failed downloaded server through the manager,
+  // which stops that process before deleting its resources.
+  virtual void InvalidateDownloadedVersion() = 0;
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_SERVER_UPDATER_H_
