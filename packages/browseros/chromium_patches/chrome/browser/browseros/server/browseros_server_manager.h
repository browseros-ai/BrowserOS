diff --git a/chrome/browser/browseros/server/browseros_server_manager.h b/chrome/browser/browseros/server/browseros_server_manager.h
new file mode 100644
index 0000000000000..6c9a7e03ebc0d
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_manager.h
@@ -0,0 +1,167 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_MANAGER_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_MANAGER_H_
+
+#include <memory>
+#include <set>
+
+#include "base/files/file.h"
+#include "base/files/file_path.h"
+#include "base/functional/callback.h"
+#include "base/memory/raw_ptr.h"
+#include "base/memory/weak_ptr.h"
+#include "base/no_destructor.h"
+#include "base/process/process.h"
+#include "base/timer/timer.h"
+#include "chrome/browser/browseros/server/browseros_server_config.h"
+#include "chrome/browser/browseros/server/process_controller.h"
+
+class PrefChangeRegistrar;
+class PrefService;
+
+namespace browseros_server {
+class BrowserOSServerUpdater;
+}
+
+namespace browseros {
+class BrowserOSServerProxy;
+class HealthChecker;
+class ProcessController;
+class ServerStateStore;
+class ServerUpdater;
+}  // namespace browseros
+
+namespace browseros {
+
+// Manages the selected BrowserOS product server process.
+class BrowserOSServerManager {
+ public:
+  // Production singleton (uses real implementations)
+  static BrowserOSServerManager* GetInstance();
+
+  // Test constructor (dependency injection)
+  BrowserOSServerManager(std::unique_ptr<ProcessController> process_controller,
+                         std::unique_ptr<ServerStateStore> state_store,
+                         std::unique_ptr<HealthChecker> health_checker,
+                         std::unique_ptr<ServerUpdater> updater,
+                         PrefService* local_state);
+
+  BrowserOSServerManager(const BrowserOSServerManager&) = delete;
+  BrowserOSServerManager& operator=(const BrowserOSServerManager&) = delete;
+
+  void Start();
+  void Stop();
+  bool IsRunning() const;
+
+  // Returns the stable MCP proxy port (what external clients connect to)
+  int GetMCPPort() const { return ports_.proxy; }
+  int GetProxyPort() const { return ports_.proxy; }
+
+  int GetCDPPort() const { return ports_.cdp; }
+  int GetServerPort() const { return ports_.server; }
+
+  const ServerPorts& GetPorts() const { return ports_; }
+
+  bool IsAllowRemoteInMCP() const { return allow_remote_in_mcp_; }
+
+  void Shutdown();
+
+  // Health check result handler (public for testing)
+  void OnHealthCheckComplete(bool success);
+
+  void SetRunningForTesting(bool running) { is_running_ = running; }
+  void SetPortsForTesting(const ServerPorts& ports) { ports_ = ports; }
+  using PortFinderForTesting =
+      base::RepeatingCallback<int(int, const std::set<int>&, bool)>;
+  void SetPortFinderForTesting(PortFinderForTesting port_finder) {
+    port_finder_for_testing_ = port_finder;
+  }
+  void OnProcessExitedForTesting(int exit_code) { OnProcessExited(exit_code); }
+
+  base::FilePath GetBrowserOSServerExecutablePath() const;
+  base::FilePath GetBrowserOSServerResourcesPath() const;
+
+  using UpdateCompleteCallback = base::OnceCallback<void(bool success)>;
+  void RestartServerForUpdate(UpdateCompleteCallback callback);
+
+ private:
+  friend base::NoDestructor<BrowserOSServerManager>;
+
+  BrowserOSServerManager();
+  ~BrowserOSServerManager();
+
+  bool AcquireLock();
+  bool RecoverFromOrphan();
+
+  void LoadPortsFromPrefs();
+  void SetupPrefObservers();
+  void ResolvePortsForStartup();
+  void ApplyCommandLineOverrides();
+  void SavePortsToPrefs();
+  int FindAvailableServerPort(int starting_port,
+                              const std::set<int>& excluded,
+                              bool allow_reuse);
+  void StartCDPServer();
+  void StopCDPServer();
+  void StartProxy();
+  void StopProxy();
+
+  ServerLaunchConfig BuildLaunchConfig();
+
+  void LaunchBrowserOSProcess();
+  void OnProcessLaunched(LaunchResult result);
+
+  void TerminateBrowserOSProcess(base::OnceCallback<void()> callback);
+  void OnTerminateProcessComplete(base::OnceCallback<void()> callback,
+                                  bool killed);
+
+  void RestartBrowserOSProcess();
+  void ContinueRestartAfterTerminate();
+  void ContinueUpdateAfterTerminate();
+
+  void OnProcessExited(int exit_code);
+  void CheckServerHealth();
+  void OnAllowRemoteInMCPChanged();
+  void OnProxyPortChanged();
+  void OnRestartServerRequestedChanged();
+  void CheckProcessStatus();
+
+  base::FilePath GetBrowserOSExecutionDir() const;
+
+  std::unique_ptr<ProcessController> process_controller_;
+  std::unique_ptr<ServerStateStore> state_store_;
+  std::unique_ptr<HealthChecker> health_checker_;
+  std::unique_ptr<BrowserOSServerProxy> server_proxy_;
+
+  raw_ptr<PrefService> local_state_ = nullptr;
+
+  base::File lock_file_;
+  base::Process process_;
+  ServerPorts ports_;
+  bool allow_remote_in_mcp_ = false;
+  bool is_running_ = false;
+  bool is_restarting_ = false;
+  bool is_updating_ = false;
+  UpdateCompleteCallback update_complete_callback_;
+
+  int consecutive_startup_failures_ = 0;
+  int consecutive_health_failures_ = 0;
+  bool advance_port_on_restart_ = false;
+  base::TimeTicks last_launch_time_;
+
+  base::RepeatingTimer health_check_timer_;
+  base::RepeatingTimer process_check_timer_;
+
+  std::unique_ptr<PrefChangeRegistrar> pref_change_registrar_;
+  std::unique_ptr<ServerUpdater> updater_;
+  PortFinderForTesting port_finder_for_testing_;
+
+  base::WeakPtrFactory<BrowserOSServerManager> weak_factory_{this};
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_MANAGER_H_
