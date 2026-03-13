diff --git a/chrome/browser/win/winsparkle_glue.h b/chrome/browser/win/winsparkle_glue.h
new file mode 100644
index 0000000000000..850df8ff9fb41
--- /dev/null
+++ b/chrome/browser/win/winsparkle_glue.h
@@ -0,0 +1,96 @@
+// Copyright 2024 BrowserOS Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_WIN_WINSPARKLE_GLUE_H_
+#define CHROME_BROWSER_WIN_WINSPARKLE_GLUE_H_
+
+#include <string>
+
+#include "base/functional/callback.h"
+#include "base/observer_list.h"
+#include "base/observer_list_types.h"
+#include "base/sequence_checker.h"
+
+namespace winsparkle_glue {
+
+// WinSparkle updater status.
+enum class WinSparkleStatus {
+  kIdle = 0,
+  kChecking,
+  kUpdateFound,
+  kUpToDate,
+  kError,
+};
+
+// Observer interface for WinSparkle status changes.
+class WinSparkleObserver : public base::CheckedObserver {
+ public:
+  virtual void OnWinSparkleStatusChanged(WinSparkleStatus status) = 0;
+};
+
+// Main interface for WinSparkle integration.
+// Thread-safety: All public methods must be called on the UI thread.
+// WinSparkle callbacks arrive on background threads and are posted
+// to the UI thread before observer notification.
+class WinSparkleGlue {
+ public:
+  static WinSparkleGlue* GetInstance();
+
+  WinSparkleGlue(const WinSparkleGlue&) = delete;
+  WinSparkleGlue& operator=(const WinSparkleGlue&) = delete;
+
+  // Initialize WinSparkle. Returns true if initialized successfully.
+  bool Initialize();
+
+  // Shut down WinSparkle. Must be called before process exit.
+  void Shutdown();
+
+  // Returns true if WinSparkle is initialized and operational.
+  bool IsInitialized() const;
+
+  // Trigger a manual update check (shows progress UI).
+  void CheckForUpdates();
+
+  // Current status.
+  WinSparkleStatus status() const;
+
+  // Observer management.
+  void AddObserver(WinSparkleObserver* observer);
+  void RemoveObserver(WinSparkleObserver* observer);
+
+ private:
+  WinSparkleGlue();
+  ~WinSparkleGlue();
+
+  // Configure WinSparkle before initialization.
+  void ConfigureAppDetails();
+  void ConfigureAppcastUrl();
+  void ConfigureCallbacks();
+
+  // Static callbacks invoked by WinSparkle from background threads.
+  static void __cdecl OnError();
+  static int __cdecl OnCanShutdown();
+  static void __cdecl OnShutdownRequest();
+  static void __cdecl OnDidFindUpdate();
+  static void __cdecl OnDidNotFindUpdate();
+  static void __cdecl OnUpdateDismissed();
+
+  // Notify observers of status change (must be on UI thread).
+  void NotifyStatusChanged(WinSparkleStatus new_status);
+
+  bool initialized_ = false;
+  WinSparkleStatus status_ = WinSparkleStatus::kIdle;
+  base::ObserverList<WinSparkleObserver> observers_;
+
+  SEQUENCE_CHECKER(sequence_checker_);
+};
+
+// Convenience functions for non-class code.
+
+// Returns true if WinSparkle is enabled and initialized.
+bool WinSparkleEnabled();
+
+}  // namespace winsparkle_glue
+
+#endif  // CHROME_BROWSER_WIN_WINSPARKLE_GLUE_H_
