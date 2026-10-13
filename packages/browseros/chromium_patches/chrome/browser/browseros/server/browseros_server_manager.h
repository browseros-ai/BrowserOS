diff --git a/chrome/browser/browseros/server/browseros_server_manager.h b/chrome/browser/browseros/server/browseros_server_manager.h
new file mode 100644
index 0000000000000000000000000000000000000000..dd0a2cf2cf09faba02600b569c492916a251ce6e
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_manager.h
@@ -0,0 +1,249 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_MANAGER_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_MANAGER_H_
+
+#include <cstdint>
+#include <memory>
+#include <optional>
+#include <set>
+#include <string>
+#include <string_view>
+
+#include "base/files/file.h"
+#include "base/files/file_path.h"
+#include "base/functional/callback.h"
+#include "base/memory/raw_ptr.h"
+#include "base/memory/weak_ptr.h"
+#include "base/no_destructor.h"
+#include "base/process/process.h"
+#include "base/timer/timer.h"
+#include "base/values.h"
+#include "chrome/browser/browseros/server/browseros_server_config.h"
+#include "chrome/browser/browseros/server/browseros_server_prefs.h"
+#include "chrome/browser/browseros/server/process_controller.h"
+#include "chrome/browser/browseros/server/server_version_store.h"
+
+class PrefChangeRegistrar;
+class PrefService;
+
+namespace network {
+class SimpleURLLoader;
+}
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
+  ServerVersionStore& GetVersionStore();
+  base::Version GetRunningVersion() const;
+  // Switches to the store's ready download. With a healthy server this waits
+  // for its can_update; with none it activates immediately.
+  void MaybeActivateReadyVersion();
+  // Rejects the launched download at once, skipping the remaining strikes.
+  void InvalidateDownloadedServer();
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
+  void OnVersionsInitialized();
+  void OnVersionStoreStopped();
+  void FinishStop();
+  void ClearRunningInstallation();
+  void OnStartupHealthTimeout(uint64_t generation);
+
+  // Supervision. Every launched-installation failure funnels into OnStrike(),
+  // which restarts it and, after kMaxStrikes, switches installation. See the
+  // policy comment in the .cc.
+  void OnStrike(std::string_view reason);
+  void SwitchAwayFromLaunchedInstallation(std::string_view reason);
+  void ForgiveStrikes();
+  void RelaunchAfterFailure(base::TimeDelta delay);
+  void ScheduleRelaunch(base::TimeDelta delay);
+  void ActivateWithoutHealthyServer(const ServerInstallation& target);
+  // Sends a supervision event at most once per event and version per session.
+  void LogSupervisionEvent(std::string_view event,
+                           const base::Version& version,
+                           base::DictValue properties);
+
+  void CompleteUpdate(bool success);
+  void StartUpdater();
+  void OnReadinessChecked(ServerInstallation installation,
+                          uint64_t generation,
+                          std::optional<std::string> response);
+  void ActivateInstallation(const ServerInstallation& installation);
+  void OnReadyVersionActivated(const base::Version& old_version,
+                               const base::Version& new_version,
+                               bool success);
+
+  void TerminateBrowserOSProcess(base::OnceCallback<void()> callback);
+  void OnTerminateProcessComplete(bool killed);
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
+  // Background launches retain the controller even if the manager is destroyed.
+  std::shared_ptr<ProcessController> process_controller_;
+  std::unique_ptr<ServerStateStore> state_store_;
+  std::unique_ptr<HealthChecker> health_checker_;
+  std::unique_ptr<BrowserOSServerProxy> server_proxy_;
+
+  raw_ptr<PrefService> local_state_ = nullptr;
+
+  base::File lock_file_;
+  base::Process process_;
+  ServerPorts ports_;
+  int proxy_https_port_ = browseros_server::kDefaultProxyHttpsPort;
+  bool allow_remote_in_mcp_ = false;
+  bool is_running_ = false;
+  bool is_restarting_ = false;
+  bool is_updating_ = false;
+  UpdateCompleteCallback update_complete_callback_;
+  base::OnceClosure termination_callback_;
+
+  // Selection precedes launch; running identity is published only after health.
+  // Ready state remains in the store while activation is deferred or in flight.
+  std::unique_ptr<ServerVersionStore> version_store_;
+  std::optional<ServerInstallation> launched_installation_;
+  std::optional<ServerInstallation> running_installation_;
+  std::optional<ServerInstallation> next_installation_;
+  bool started_ = false;
+  bool stopping_ = false;
+  bool launch_in_progress_ = false;
+  bool termination_in_progress_ = false;
+  bool health_check_in_progress_ = false;
+  bool updater_started_ = false;
+  bool version_io_pending_ = false;
+  uint64_t process_generation_ = 0;
+
+  // Failures of the launched installation since it was last stably healthy
+  // or a new version was activated.
+  int strikes_ = 0;
+  // Installations that ran out of strikes in a row; a full set means every
+  // option failed and the next relaunch waits.
+  int failed_installations_ = 0;
+  // Start of the current process's unbroken run of passing health checks;
+  // null after a missed check or a new launch.
+  base::TimeTicks healthy_since_;
+  int consecutive_health_failures_ = 0;
+  bool advance_port_on_restart_ = false;
+  // "<event>:<version>" keys already reported this session.
+  std::set<std::string> reported_supervision_events_;
+
+  base::RepeatingTimer health_check_timer_;
+  base::OneShotTimer relaunch_timer_;
+  base::RepeatingTimer process_check_timer_;
+  base::OneShotTimer startup_health_retry_timer_;
+  base::OneShotTimer startup_health_deadline_timer_;
+  base::RepeatingTimer activation_retry_timer_;
+  std::unique_ptr<network::SimpleURLLoader> readiness_loader_;
+
+  std::unique_ptr<PrefChangeRegistrar> pref_change_registrar_;
+  std::unique_ptr<ServerUpdater> updater_;
+  PortFinderForTesting port_finder_for_testing_;
+
+  base::WeakPtrFactory<BrowserOSServerManager> weak_factory_{this};
+  // Launch/termination replies must survive Stop() so their child can be reaped
+  // before the profile lock is released. Other lifecycle replies are cancelled.
+  base::WeakPtrFactory<BrowserOSServerManager> process_weak_factory_{this};
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_MANAGER_H_
