diff --git a/chrome/browser/win/winsparkle_glue.cc b/chrome/browser/win/winsparkle_glue.cc
new file mode 100644
index 0000000000000..869ed71be8869
--- /dev/null
+++ b/chrome/browser/win/winsparkle_glue.cc
@@ -0,0 +1,351 @@
+// Copyright 2024 BrowserOS Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/win/winsparkle_glue.h"
+
+#include <windows.h>
+
+#include <stdint.h>
+
+#include <string>
+#include <vector>
+
+#include "base/command_line.h"
+#include "base/files/file_path.h"
+#include "base/functional/bind.h"
+#include "base/logging.h"
+#include "base/no_destructor.h"
+#include "base/observer_list.h"
+#include "base/path_service.h"
+#include "base/process/launch.h"
+#include "base/process/process.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/strings/utf_string_conversions.h"
+#include "base/task/single_thread_task_runner.h"
+#include "base/task/thread_pool.h"
+#include "base/time/time.h"
+#include "base/version.h"
+#include "base/version_info/version_info.h"
+#include "build/build_config.h"
+#include "content/public/browser/browser_thread.h"
+#include "third_party/winsparkle/include/winsparkle.h"
+
+namespace winsparkle_glue {
+
+namespace {
+
+// Same Ed25519 public key as the macOS Sparkle build (SUPublicEDKey in
+// chrome/app/app-Info.plist); the BrowserOS release pipeline signs all
+// platforms' artifacts with the one key.
+constexpr char kEdDSAPublicKey[] =
+    "LzQmcNuTsdB3/dsivo0eeN+jPfDoriRHAkkEJcfFs2A=";
+
+// Windows builds are single-arch, so the feed is chosen at compile time
+// (macOS picks at runtime because of universal binaries).
+#if defined(ARCH_CPU_ARM64)
+constexpr char kAppcastURL[] =
+    "https://cdn.browseros.com/appcast-win-arm64.xml";
+#else
+constexpr char kAppcastURL[] = "https://cdn.browseros.com/appcast-win.xml";
+#endif
+
+// Matches SUScheduledCheckInterval on macOS; also WinSparkle's minimum.
+constexpr int kUpdateCheckIntervalSeconds = 3600;
+
+constexpr char kRegistryPath[] = "Software\\BrowserOS\\WinSparkle";
+
+// Owns WinSparkle state for the browser process. All public methods are UI
+// thread only; Notify* run on the UI thread via PostToUI from WinSparkle's
+// worker threads.
+class WinSparkleGlue {
+ public:
+  static WinSparkleGlue* GetInstance() {
+    static base::NoDestructor<WinSparkleGlue> instance;
+    return instance.get();
+  }
+
+  bool Initialize();
+  bool enabled() const { return enabled_; }
+  void Cleanup();
+
+  void AddObserver(WinSparkleObserver* observer) {
+    DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+    observers_.AddObserver(observer);
+  }
+
+  void RemoveObserver(WinSparkleObserver* observer) {
+    DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+    observers_.RemoveObserver(observer);
+  }
+
+  // Thread-safe; tasks posted after shutdown are dropped by the task runner.
+  void PostToUI(base::OnceClosure task) {
+    if (ui_task_runner_) {
+      ui_task_runner_->PostTask(FROM_HERE, std::move(task));
+    }
+  }
+
+  // Launches the downloaded installer. Called on WinSparkle's thread; the
+  // launch happens synchronously there so a browser shutdown racing this
+  // callback cannot drop a user-confirmed install — only the wait for the
+  // installer to finish moves to the thread pool. mini_installer supports
+  // in-use updates (it stages the new version next to the running one), so
+  // the browser keeps running and the standard relaunch-to-update flow
+  // finishes the job.
+  bool LaunchInstaller(const base::FilePath& installer_path) {
+    base::LaunchOptions options;
+    options.start_hidden = true;
+    base::Process process =
+        base::LaunchProcess(base::CommandLine(installer_path), options);
+    if (!process.IsValid()) {
+      LOG(ERROR) << "WinSparkle: failed to launch installer "
+                 << installer_path.value();
+      PostToUI(base::BindOnce(&WinSparkleGlue::NotifyUpdateError,
+                              base::Unretained(this)));
+      return false;
+    }
+    base::ThreadPool::PostTask(
+        FROM_HERE,
+        {base::MayBlock(), base::TaskPriority::BEST_EFFORT,
+         base::TaskShutdownBehavior::CONTINUE_ON_SHUTDOWN},
+        base::BindOnce(&WinSparkleGlue::WaitForInstaller, std::move(process)));
+    return true;
+  }
+
+  void NotifyUpdateFound() {
+    DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+    for (WinSparkleObserver& observer : observers_) {
+      observer.OnUpdateFound();
+    }
+  }
+
+  void NotifyNoUpdateFound() {
+    DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+    for (WinSparkleObserver& observer : observers_) {
+      observer.OnNoUpdateFound();
+    }
+  }
+
+  void NotifyUpdateCancelled() {
+    DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+    for (WinSparkleObserver& observer : observers_) {
+      observer.OnUpdateCancelled();
+    }
+  }
+
+  void NotifyUpdateInstalled() {
+    DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+    for (WinSparkleObserver& observer : observers_) {
+      observer.OnUpdateInstalled();
+    }
+  }
+
+  void NotifyUpdateError() {
+    DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+    for (WinSparkleObserver& observer : observers_) {
+      observer.OnUpdateError();
+    }
+  }
+
+ private:
+  friend class base::NoDestructor<WinSparkleGlue>;
+  WinSparkleGlue() = default;
+
+  static void WaitForInstaller(base::Process process);
+
+  bool initialized_ = false;
+  bool enabled_ = false;
+  scoped_refptr<base::SingleThreadTaskRunner> ui_task_runner_;
+  base::ObserverList<WinSparkleObserver> observers_;
+};
+
+// WinSparkle C callbacks. Invoked on WinSparkle's worker threads — never on
+// the browser UI thread — so everything is marshalled through PostToUI.
+
+void __cdecl OnErrorCallback() {
+  WinSparkleGlue* glue = WinSparkleGlue::GetInstance();
+  glue->PostToUI(base::BindOnce(&WinSparkleGlue::NotifyUpdateError,
+                                base::Unretained(glue)));
+}
+
+void __cdecl OnDidFindUpdateCallback() {
+  WinSparkleGlue* glue = WinSparkleGlue::GetInstance();
+  glue->PostToUI(base::BindOnce(&WinSparkleGlue::NotifyUpdateFound,
+                                base::Unretained(glue)));
+}
+
+void __cdecl OnDidNotFindUpdateCallback() {
+  WinSparkleGlue* glue = WinSparkleGlue::GetInstance();
+  glue->PostToUI(base::BindOnce(&WinSparkleGlue::NotifyNoUpdateFound,
+                                base::Unretained(glue)));
+}
+
+void __cdecl OnUpdateCancelledCallback() {
+  WinSparkleGlue* glue = WinSparkleGlue::GetInstance();
+  glue->PostToUI(base::BindOnce(&WinSparkleGlue::NotifyUpdateCancelled,
+                                base::Unretained(glue)));
+}
+
+int __cdecl OnUserRunInstallerCallback(const wchar_t* installer_path) {
+  if (!installer_path || !installer_path[0]) {
+    return WINSPARKLE_RETURN_ERROR;
+  }
+  // 1 = handled: WinSparkle must not run the installer itself. A failed
+  // launch returns an error so WinSparkle's dialog reports it instead of
+  // silently closing.
+  return WinSparkleGlue::GetInstance()->LaunchInstaller(
+             base::FilePath(installer_path))
+             ? 1
+             : WINSPARKLE_RETURN_ERROR;
+}
+
+// static
+void WinSparkleGlue::WaitForInstaller(base::Process process) {
+  WinSparkleGlue* glue = GetInstance();
+
+  int exit_code = 0;
+  if (!process.WaitForExitWithTimeout(base::Minutes(10), &exit_code)) {
+    LOG(ERROR) << "WinSparkle: timed out waiting for the installer";
+    glue->PostToUI(base::BindOnce(&WinSparkleGlue::NotifyUpdateError,
+                                  base::Unretained(glue)));
+    return;
+  }
+  if (exit_code != 0) {
+    LOG(ERROR) << "WinSparkle: installer exited with code " << exit_code;
+    glue->PostToUI(base::BindOnce(&WinSparkleGlue::NotifyUpdateError,
+                                  base::Unretained(glue)));
+    return;
+  }
+
+  VLOG(1) << "WinSparkle: installer completed; update pending relaunch";
+  glue->PostToUI(base::BindOnce(&WinSparkleGlue::NotifyUpdateInstalled,
+                                base::Unretained(glue)));
+}
+
+bool WinSparkleGlue::Initialize() {
+  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+  if (initialized_) {
+    return enabled_;
+  }
+  initialized_ = true;
+
+  // Same kill switch as the macOS Sparkle glue; keeps dev runs and tests
+  // from hitting the production appcast.
+  auto* command_line = base::CommandLine::ForCurrentProcess();
+  if (command_line && command_line->HasSwitch("disable-updates")) {
+    VLOG(1) << "WinSparkle: updates disabled via command line";
+    return false;
+  }
+
+  base::FilePath module_dir;
+  if (!base::PathService::Get(base::DIR_MODULE, &module_dir)) {
+    return false;
+  }
+  base::FilePath dll_path =
+      module_dir.Append(FILE_PATH_LITERAL("WinSparkle.dll"));
+
+  // Explicit absolute-path load. chrome.dll only delay-loads the import, so
+  // resolving it here (a) keeps the standard DLL search path out of the
+  // picture and (b) turns a missing DLL into a disabled updater instead of a
+  // delay-load crash on first call. Dependencies may resolve only from the
+  // DLL's own directory and system32.
+  HMODULE module = ::LoadLibraryExW(
+      dll_path.value().c_str(), nullptr,
+      LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32);
+  if (!module) {
+    VLOG(1) << "WinSparkle: " << dll_path.value()
+            << " not found; updater disabled";
+    return false;
+  }
+
+  ui_task_runner_ = content::GetUIThreadTaskRunner({});
+
+  win_sparkle_set_appcast_url(kAppcastURL);
+  if (!win_sparkle_set_eddsa_public_key(kEdDSAPublicKey)) {
+    LOG(ERROR) << "WinSparkle: invalid EdDSA public key; updater disabled";
+    return false;
+  }
+
+  // Display version for WinSparkle UI / User-Agent; comparisons use the
+  // BUILD.PATCH build version below, which is what the appcast carries in
+  // sparkle:version (same scheme as CFBundleVersion on macOS).
+  const std::wstring display_version =
+      base::UTF8ToWide(version_info::GetVersionNumber());
+  win_sparkle_set_app_details(L"BrowserOS", L"BrowserOS",
+                              display_version.c_str());
+
+  const std::vector<uint32_t>& components =
+      version_info::GetVersion().components();
+  if (components.size() >= 4) {
+    const std::wstring build_version =
+        base::UTF8ToWide(base::NumberToString(components[2]) + "." +
+                         base::NumberToString(components[3]));
+    win_sparkle_set_app_build_version(build_version.c_str());
+  }
+
+  win_sparkle_set_registry_path(kRegistryPath);
+
+  // Pre-seeding the automatic-check setting keeps WinSparkle from showing
+  // its first-run permission prompt (macOS equivalent: SUEnableAutomaticChecks
+  // plus the auto-granting user driver).
+  win_sparkle_set_automatic_check_for_updates(1);
+  win_sparkle_set_update_check_interval(kUpdateCheckIntervalSeconds);
+
+  // can_shutdown / shutdown_request stay unregistered on purpose: WinSparkle
+  // 0.9.3 calls the shutdown request unconditionally after the run-installer
+  // callback reports "handled" (ui.cpp OnRunInstaller), and the unregistered
+  // default is a no-op — exactly right for the background-install flow where
+  // the browser must keep running.
+  win_sparkle_set_error_callback(&OnErrorCallback);
+  win_sparkle_set_did_find_update_callback(&OnDidFindUpdateCallback);
+  win_sparkle_set_did_not_find_update_callback(&OnDidNotFindUpdateCallback);
+  win_sparkle_set_update_cancelled_callback(&OnUpdateCancelledCallback);
+  win_sparkle_set_update_skipped_callback(&OnUpdateCancelledCallback);
+  win_sparkle_set_update_postponed_callback(&OnUpdateCancelledCallback);
+  win_sparkle_set_update_dismissed_callback(&OnUpdateCancelledCallback);
+  win_sparkle_set_user_run_installer_callback(&OnUserRunInstallerCallback);
+
+  win_sparkle_init();
+  enabled_ = true;
+  VLOG(1) << "WinSparkle: initialized, feed " << kAppcastURL;
+  return true;
+}
+
+void WinSparkleGlue::Cleanup() {
+  if (enabled_) {
+    win_sparkle_cleanup();
+    enabled_ = false;
+  }
+}
+
+}  // namespace
+
+bool Initialize() {
+  return WinSparkleGlue::GetInstance()->Initialize();
+}
+
+bool IsEnabled() {
+  return WinSparkleGlue::GetInstance()->enabled();
+}
+
+void Cleanup() {
+  WinSparkleGlue::GetInstance()->Cleanup();
+}
+
+void CheckForUpdatesWithUI() {
+  if (!IsEnabled()) {
+    return;
+  }
+  win_sparkle_check_update_with_ui();
+}
+
+void AddObserver(WinSparkleObserver* observer) {
+  WinSparkleGlue::GetInstance()->AddObserver(observer);
+}
+
+void RemoveObserver(WinSparkleObserver* observer) {
+  WinSparkleGlue::GetInstance()->RemoveObserver(observer);
+}
+
+}  // namespace winsparkle_glue
