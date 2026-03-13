diff --git a/chrome/browser/win/winsparkle_glue.cc b/chrome/browser/win/winsparkle_glue.cc
new file mode 100644
index 0000000000000..c890486392462
--- /dev/null
+++ b/chrome/browser/win/winsparkle_glue.cc
@@ -0,0 +1,292 @@
+// Copyright 2024 BrowserOS Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/win/winsparkle_glue.h"
+
+#include <windows.h>
+
+#include "base/command_line.h"
+#include "base/logging.h"
+#include "base/no_destructor.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/strings/utf_string_conversions.h"
+#include "base/task/single_thread_task_runner.h"
+#include "base/version_info/version_info.h"
+#include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_switches.h"
+#include "chrome/browser/lifetime/application_lifetime.h"
+#include "chrome/browser/upgrade_detector/build_state.h"
+#include "content/public/browser/browser_thread.h"
+
+// WinSparkle C API header. Linking is handled by BUILD.gn.
+#include "winsparkle.h"
+
+namespace winsparkle_glue {
+
+namespace {
+
+// Default appcast URL for Windows updates.
+const char kDefaultAppcastUrl[] = "https://cdn.browseros.com/appcast-win.xml";
+
+// Notify the Chromium upgrade system that a WinSparkle update is available.
+void NotifyUpgradeReady() {
+  if (!g_browser_process) {
+    LOG(WARNING) << "WinSparkle: Cannot notify upgrade - no browser process";
+    return;
+  }
+
+  BuildState* build_state = g_browser_process->GetBuildState();
+  if (!build_state) {
+    LOG(WARNING) << "WinSparkle: Cannot notify upgrade - no build state";
+    return;
+  }
+
+  VLOG(1) << "WinSparkle: Notifying upgrade system";
+  // WinSparkle does not expose the found version in its callback API,
+  // so we pass a placeholder. The actual version is shown by WinSparkle's
+  // own update dialog.
+  build_state->SetUpdate(BuildState::UpdateType::kNormalUpdate,
+                         base::Version("0.0.0.0"), std::nullopt);
+}
+
+// Post a task to the UI thread if not already on it.
+void PostToUIThread(base::OnceClosure task) {
+  if (content::BrowserThread::CurrentlyOn(content::BrowserThread::UI)) {
+    std::move(task).Run();
+    return;
+  }
+  content::GetUIThreadTaskRunner({})->PostTask(FROM_HERE, std::move(task));
+}
+
+}  // namespace
+
+// static
+WinSparkleGlue* WinSparkleGlue::GetInstance() {
+  static base::NoDestructor<WinSparkleGlue> instance;
+  return instance.get();
+}
+
+WinSparkleGlue::WinSparkleGlue() {
+  DETACH_FROM_SEQUENCE(sequence_checker_);
+}
+
+// NoDestructor means this never runs; Shutdown() is called explicitly
+// in PostMainMessageLoopRun.
+WinSparkleGlue::~WinSparkleGlue() = default;
+
+bool WinSparkleGlue::Initialize() {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+
+  if (initialized_) {
+    return true;
+  }
+
+  auto* cmd = base::CommandLine::ForCurrentProcess();
+  if (cmd && cmd->HasSwitch("disable-updates")) {
+    VLOG(1) << "WinSparkle: Updates disabled via command line";
+    return false;
+  }
+
+  ConfigureAppDetails();
+  ConfigureAppcastUrl();
+  ConfigureCallbacks();
+
+  // Enable automatic update checks every hour.
+  win_sparkle_set_automatic_check_for_updates(1);
+  win_sparkle_set_update_check_interval(3600);
+
+  // Initialize WinSparkle (non-blocking).
+  win_sparkle_init();
+  initialized_ = true;
+
+  VLOG(1) << "WinSparkle: Initialized successfully";
+
+  // If force check requested, trigger immediately.
+  if (cmd && cmd->HasSwitch(browseros::kWinSparkleForceCheck)) {
+    VLOG(1) << "WinSparkle: Force check triggered via command line";
+    CheckForUpdates();
+  }
+
+  return true;
+}
+
+void WinSparkleGlue::Shutdown() {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+
+  if (!initialized_) {
+    return;
+  }
+
+  VLOG(1) << "WinSparkle: Shutting down";
+  initialized_ = false;
+  win_sparkle_cleanup();
+}
+
+bool WinSparkleGlue::IsInitialized() const {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+  return initialized_;
+}
+
+void WinSparkleGlue::CheckForUpdates() {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+
+  if (!initialized_) {
+    LOG(WARNING) << "WinSparkle: Cannot check - not initialized";
+    return;
+  }
+
+  VLOG(1) << "WinSparkle: Checking for updates";
+  NotifyStatusChanged(WinSparkleStatus::kChecking);
+
+  win_sparkle_check_update_with_ui();
+}
+
+WinSparkleStatus WinSparkleGlue::status() const {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+  return status_;
+}
+
+void WinSparkleGlue::AddObserver(WinSparkleObserver* observer) {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+  observers_.AddObserver(observer);
+}
+
+void WinSparkleGlue::RemoveObserver(WinSparkleObserver* observer) {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+  observers_.RemoveObserver(observer);
+}
+
+void WinSparkleGlue::ConfigureAppDetails() {
+  auto* cmd = base::CommandLine::ForCurrentProcess();
+
+  std::wstring version =
+      base::UTF8ToWide(version_info::GetVersionNumber());
+
+  // Allow version spoofing for testing.
+  if (cmd && cmd->HasSwitch(browseros::kWinSparkleSpoofVersion)) {
+    std::string spoofed =
+        cmd->GetSwitchValueASCII(browseros::kWinSparkleSpoofVersion);
+    LOG(WARNING) << "WinSparkle: Spoofing version as " << spoofed;
+    version = base::UTF8ToWide(spoofed);
+  }
+
+  win_sparkle_set_app_details(L"BrowserOS", L"BrowserOS", version.c_str());
+  win_sparkle_set_app_build_version(version.c_str());
+
+  VLOG(1) << "WinSparkle: App version set to "
+          << base::WideToUTF8(version);
+}
+
+void WinSparkleGlue::ConfigureAppcastUrl() {
+  auto* cmd = base::CommandLine::ForCurrentProcess();
+
+  std::string url;
+  if (cmd && cmd->HasSwitch(browseros::kWinSparkleUrl)) {
+    url = cmd->GetSwitchValueASCII(browseros::kWinSparkleUrl);
+    LOG(WARNING) << "WinSparkle: Using override URL: " << url;
+  } else {
+    url = kDefaultAppcastUrl;
+  }
+
+  win_sparkle_set_appcast_url(url.c_str());
+  VLOG(1) << "WinSparkle: Appcast URL set to " << url;
+}
+
+void WinSparkleGlue::ConfigureCallbacks() {
+  win_sparkle_set_error_callback(&WinSparkleGlue::OnError);
+  win_sparkle_set_can_shutdown_callback(&WinSparkleGlue::OnCanShutdown);
+  win_sparkle_set_shutdown_request_callback(
+      &WinSparkleGlue::OnShutdownRequest);
+  win_sparkle_set_did_find_update_callback(&WinSparkleGlue::OnDidFindUpdate);
+  win_sparkle_set_did_not_find_update_callback(
+      &WinSparkleGlue::OnDidNotFindUpdate);
+  // Register dismissed callback to reset status when user closes
+  // the WinSparkle dialog (cancel, skip, remind later, etc.)
+  win_sparkle_set_update_dismissed_callback(
+      &WinSparkleGlue::OnUpdateDismissed);
+}
+
+// static - Called from WinSparkle background thread.
+void __cdecl WinSparkleGlue::OnError() {
+  LOG(ERROR) << "WinSparkle: Update error";
+  PostToUIThread(base::BindOnce(
+      [](WinSparkleGlue* glue) {
+        if (!glue->initialized_) {
+          return;
+        }
+        glue->NotifyStatusChanged(WinSparkleStatus::kError);
+      },
+      GetInstance()));
+}
+
+// static - Called from WinSparkle background thread.
+int __cdecl WinSparkleGlue::OnCanShutdown() {
+  // Always allow shutdown. Chrome's shutdown sequence handles cleanup.
+  return 1;
+}
+
+// static - Called from WinSparkle background thread.
+void __cdecl WinSparkleGlue::OnShutdownRequest() {
+  VLOG(1) << "WinSparkle: Shutdown requested for update installation";
+  PostToUIThread(base::BindOnce([]() {
+    // Use AttemptRestart for a graceful restart that gives the user
+    // a chance to save work, rather than EndSession which is abrupt.
+    chrome::AttemptRestart();
+  }));
+}
+
+// static - Called from WinSparkle background thread.
+void __cdecl WinSparkleGlue::OnDidFindUpdate() {
+  VLOG(1) << "WinSparkle: Update found";
+  PostToUIThread(base::BindOnce(
+      [](WinSparkleGlue* glue) {
+        if (!glue->initialized_) {
+          return;
+        }
+        glue->NotifyStatusChanged(WinSparkleStatus::kUpdateFound);
+        NotifyUpgradeReady();
+      },
+      GetInstance()));
+}
+
+// static - Called from WinSparkle background thread.
+void __cdecl WinSparkleGlue::OnDidNotFindUpdate() {
+  VLOG(1) << "WinSparkle: No update found";
+  PostToUIThread(base::BindOnce(
+      [](WinSparkleGlue* glue) {
+        if (!glue->initialized_) {
+          return;
+        }
+        glue->NotifyStatusChanged(WinSparkleStatus::kUpToDate);
+      },
+      GetInstance()));
+}
+
+// static - Called from WinSparkle background thread.
+void __cdecl WinSparkleGlue::OnUpdateDismissed() {
+  VLOG(1) << "WinSparkle: Update dialog dismissed";
+  PostToUIThread(base::BindOnce(
+      [](WinSparkleGlue* glue) {
+        if (!glue->initialized_) {
+          return;
+        }
+        glue->NotifyStatusChanged(WinSparkleStatus::kIdle);
+      },
+      GetInstance()));
+}
+
+void WinSparkleGlue::NotifyStatusChanged(WinSparkleStatus new_status) {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+  status_ = new_status;
+  VLOG(1) << "WinSparkle: Status changed to " << static_cast<int>(new_status);
+  for (auto& observer : observers_) {
+    observer.OnWinSparkleStatusChanged(new_status);
+  }
+}
+
+bool WinSparkleEnabled() {
+  return WinSparkleGlue::GetInstance()->IsInitialized();
+}
+
+}  // namespace winsparkle_glue
