diff --git a/chrome/browser/browseros/server/browseros_server_manager.h b/chrome/browser/browseros/server/browseros_server_manager.h
new file mode 100644
index 0000000000000..457ee9e0295a3
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_manager.h
@@ -0,0 +1,212 @@
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
+class HealthChecker;
+class ProcessController;
+class ServerStateStore;
+class ServerUpdater;
+}
+
+namespace browseros {
+
+// BrowserOS: Manages the lifecycle of the BrowserOS server process (singleton)
+// This manager:
+// 1. Starts Chromium's CDP WebSocket server (port 9222+, auto-discovered)
+// 2. Launches the bundled BrowserOS server binary with CDP and MCP ports
+// 3. Monitors MCP server health via HTTP /health endpoint and auto-restarts
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
+  // Starts the BrowserOS server if not already running
+  // This will:
+  // 1. Find available CDP port (starting from 9222 or saved pref)
+  // 2. Start CDP WebSocket server on discovered port
+  // 3. Find available MCP port (starting from 9223 or saved pref)
+  // 4. Launch browseros_server binary with discovered ports
+  void Start();
+
+  // Stops the BrowserOS server
+  void Stop();
+
+  // Returns true if the server is running
+  bool IsRunning() const;
+
+  // Gets the CDP port (auto-discovered, stable across restarts)
+  int GetCDPPort() const { return ports_.cdp; }
+
+  // Gets the MCP port (auto-discovered, stable across restarts)
+  int GetMCPPort() const { return ports_.mcp; }
+
+  // Gets the Extension port (auto-discovered, stable across restarts)
+  int GetExtensionPort() const { return ports_.extension; }
+
+  // Gets all ports (for testing/debugging)
+  const ServerPorts& GetPorts() const { return ports_; }
+
+  // Returns whether remote connections are allowed in MCP server
+  bool IsAllowRemoteInMCP() const { return allow_remote_in_mcp_; }
+
+  // Called when browser is shutting down
+  void Shutdown();
+
+  // Gets the number of consecutive health check failures (for testing)
+  int GetConsecutiveHealthCheckFailures() const {
+    return consecutive_health_check_failures_;
+  }
+
+  // Returns whether the last restart triggered full port revalidation (for testing)
+  bool DidLastRestartRevalidateAllPorts() const {
+    return last_restart_revalidated_all_ports_;
+  }
+
+  // Health check result handler (public for testing)
+  void OnHealthCheckComplete(bool success);
+
+  // Sets running state for testing (allows OnHealthCheckComplete to execute)
+  void SetRunningForTesting(bool running) { is_running_ = running; }
+
+  // Path getters (used by updater)
+  base::FilePath GetBrowserOSServerExecutablePath() const;
+  base::FilePath GetBrowserOSServerResourcesPath() const;
+
+  // Restarts the server for an OTA update. Stops current process,
+  // then starts new process with updated binary path from updater.
+  // Callback is invoked with success/failure status.
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
+
+  // Orphan recovery - detects and kills any orphan server from a previous crash.
+  // Reads the state file, validates PID + creation_time to avoid killing wrong
+  // process if PID was reused, then kills the orphan if valid.
+  // Returns true if an orphan was found and killed.
+  bool RecoverFromOrphan();
+
+  // Port initialization for startup (called in order by Start())
+  void LoadPortsFromPrefs();       // 1. Load saved values from prefs
+  void SetupPrefObservers();       // 2. Set up pref change observers
+  void ResolvePortsForStartup();   // 3. MCP stays stable, others find available
+  void ApplyCommandLineOverrides(); // 4. Apply --cdp-port, --mcp-port, etc.
+  void SavePortsToPrefs();         // 5. Save final values to prefs
+  void StartCDPServer();
+  void StopCDPServer();
+
+  // Builds a complete launch configuration by resolving paths and identity.
+  // Called fresh before each launch since updater may change paths.
+  ServerLaunchConfig BuildLaunchConfig();
+
+  void LaunchBrowserOSProcess();
+  void OnProcessLaunched(LaunchResult result);
+  // Terminates the BrowserOS server process.
+  // If wait=true, blocks until process exits (must be called from background thread).
+  // If wait=false, just sends kill signal and returns (safe from any thread).
+  void TerminateBrowserOSProcess(bool wait);
+  void RestartBrowserOSProcess(bool revalidate_all_ports = false);
+
+  // Revalidates ports for restart (runs on background thread).
+  // CDP port is excluded (already bound by Chrome's DevTools server).
+  // If revalidate_all is true, all ports run through FindAvailablePort (PORT_CONFLICT).
+  // If false, MCP stays unchanged; only Extension is revalidated.
+  ServerPorts RevalidatePortsForRestart(const ServerPorts& current,
+                                        bool revalidate_all);
+
+  // UI thread callback after port revalidation.
+  // Updates member vars and prefs if changed, then launches process.
+  void OnPortsRevalidated(ServerPorts ports);
+
+  void OnProcessExited(int exit_code);
+  void CheckServerHealth();
+  void OnAllowRemoteInMCPChanged();
+  void OnRestartServerRequestedChanged();
+  void CheckProcessStatus();
+
+  base::FilePath GetBrowserOSExecutionDir() const;
+
+  // Dependencies (owned, injected via test constructor or created in default ctor)
+  std::unique_ptr<ProcessController> process_controller_;
+  std::unique_ptr<ServerStateStore> state_store_;
+  std::unique_ptr<HealthChecker> health_checker_;
+
+  // Not owned, can be null (injected for tests, otherwise uses g_browser_process)
+  raw_ptr<PrefService> local_state_ = nullptr;
+
+  base::File lock_file_;  // System-wide lock to ensure single instance
+  base::Process process_;
+  ServerPorts ports_;  // All server port assignments
+  bool allow_remote_in_mcp_ = false;  // Whether remote connections allowed in MCP
+  bool is_running_ = false;
+  bool is_restarting_ = false;  // Whether server is currently restarting
+  bool is_updating_ = false;    // Whether restarting for OTA update
+  UpdateCompleteCallback update_complete_callback_;
+
+  // Crash tracking for automatic rollback
+  int consecutive_startup_failures_ = 0;
+  base::TimeTicks last_launch_time_;
+
+  // Health check failure tracking - 3 consecutive failures triggers full port revalidation
+  int consecutive_health_check_failures_ = 0;
+  bool last_restart_revalidated_all_ports_ = false;  // For testing
+
+  // Timer for health checks
+  base::RepeatingTimer health_check_timer_;
+
+  // Timer for process status checks
+  base::RepeatingTimer process_check_timer_;
+
+  // Preference change registrar for monitoring pref changes
+  std::unique_ptr<PrefChangeRegistrar> pref_change_registrar_;
+
+  // Server updater for OTA updates (created lazily in OnProcessLaunched)
+  // Can be injected via test constructor as ServerUpdater interface
+  std::unique_ptr<ServerUpdater> updater_;
+
+  base::WeakPtrFactory<BrowserOSServerManager> weak_factory_{this};
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_MANAGER_H_
