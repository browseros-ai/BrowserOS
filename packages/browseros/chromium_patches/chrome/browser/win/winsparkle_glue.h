diff --git a/chrome/browser/win/winsparkle_glue.h b/chrome/browser/win/winsparkle_glue.h
new file mode 100644
index 0000000000000..98cdca2137440
--- /dev/null
+++ b/chrome/browser/win/winsparkle_glue.h
@@ -0,0 +1,53 @@
+// Copyright 2024 BrowserOS Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_WIN_WINSPARKLE_GLUE_H_
+#define CHROME_BROWSER_WIN_WINSPARKLE_GLUE_H_
+
+#include "base/observer_list_types.h"
+
+// C++ wrapper around the WinSparkle C API (//third_party/winsparkle), the
+// Windows counterpart of chrome/browser/mac/sparkle_glue.h. WinSparkle drives
+// update checks and its own consent/progress dialogs on a background thread;
+// this glue feeds the downloaded installer through a silent background
+// install so the browser never has to quit (see winsparkle_glue.cc).
+namespace winsparkle_glue {
+
+// Update events, dispatched on the UI thread.
+class WinSparkleObserver : public base::CheckedObserver {
+ public:
+  virtual void OnUpdateFound() {}
+  virtual void OnNoUpdateFound() {}
+  virtual void OnUpdateCancelled() {}
+  // The installer ran to completion in the background; the new version takes
+  // effect on relaunch (the upgrade detector badge appears independently via
+  // the installed-version poller).
+  virtual void OnUpdateInstalled() {}
+  virtual void OnUpdateError() {}
+};
+
+// Loads WinSparkle.dll from the version directory, configures it and starts
+// the update scheduler. Browser process UI thread only, call once after the
+// browser has started. Returns false and leaves the updater disabled if the
+// DLL or the signing key is unavailable.
+bool Initialize();
+
+// True if Initialize() succeeded.
+bool IsEnabled();
+
+// Shuts WinSparkle down (its UI thread; 0.9.3 leaves checker threads
+// running — upstream FIXME). Safe to call when disabled.
+void Cleanup();
+
+// Manual, user-initiated update check showing WinSparkle's progress UI.
+// Ignores a previously skipped version, per WinSparkle semantics.
+void CheckForUpdatesWithUI();
+
+// Observer registration. UI thread only.
+void AddObserver(WinSparkleObserver* observer);
+void RemoveObserver(WinSparkleObserver* observer);
+
+}  // namespace winsparkle_glue
+
+#endif  // CHROME_BROWSER_WIN_WINSPARKLE_GLUE_H_
