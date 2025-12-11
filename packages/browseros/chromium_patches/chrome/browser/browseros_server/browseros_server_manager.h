diff --git a/chrome/browser/browseros_server/browseros_server_manager.h b/chrome/browser/browseros_server/browseros_server_manager.h
new file mode 100644
index 0000000000000..9eb9a37980f11
--- /dev/null
+++ b/chrome/browser/browseros_server/browseros_server_manager.h
@@ -0,0 +1,128 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_MANAGER_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_MANAGER_H_
+
+#include <memory>
+
+#include "base/files/file.h"
+#include "base/files/file_path.h"
+#include "base/memory/ref_counted.h"
+#include "base/memory/weak_ptr.h"
+#include "base/no_destructor.h"
+#include "base/process/process.h"
+#include "base/timer/timer.h"
+
+class PrefChangeRegistrar;
+
+namespace net {
+class HttpResponseHeaders;
+}
+
+namespace network {
+class SimpleURLLoader;
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
+  static BrowserOSServerManager* GetInstance();
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
+  int GetCDPPort() const { return cdp_port_; }
+
+  // Gets the MCP port (auto-discovered, stable across restarts)
+  int GetMCPPort() const { return mcp_port_; }
+
+  // Gets the Agent port (auto-discovered, stable across restarts)
+  int GetAgentPort() const { return agent_port_; }
+
+  // Gets the Extension port (auto-discovered, stable across restarts)
+  int GetExtensionPort() const { return extension_port_; }
+
+  // Returns whether remote connections are allowed in MCP server
+  bool IsAllowRemoteInMCP() const { return allow_remote_in_mcp_; }
+
+  // Called when browser is shutting down
+  void Shutdown();
+
+ private:
+  friend base::NoDestructor<BrowserOSServerManager>;
+
+  BrowserOSServerManager();
+  ~BrowserOSServerManager();
+
+  bool AcquireLock();
+  void InitializePortsAndPrefs();
+  void SavePortsToPrefs();
+  void StartCDPServer();
+  void StopCDPServer();
+  void LaunchBrowserOSProcess();
+  void OnProcessLaunched(base::Process process);
+  void TerminateBrowserOSProcess();
+  void RestartBrowserOSProcess();
+  void OnProcessExited(int exit_code);
+  void CheckServerHealth();
+  void OnHealthCheckComplete(
+      std::unique_ptr<network::SimpleURLLoader> url_loader,
+      scoped_refptr<net::HttpResponseHeaders> headers);
+  void OnAllowRemoteInMCPChanged();
+  void OnRestartServerRequestedChanged();
+  void CheckProcessStatus();
+
+  base::FilePath GetBrowserOSServerResourcesPath() const;
+  base::FilePath GetBrowserOSExecutionDir() const;
+  base::FilePath GetBrowserOSServerExecutablePath() const;
+  int FindAvailablePort(int starting_port);
+  bool IsPortAvailable(int port);
+
+  base::File lock_file_;  // System-wide lock to ensure single instance
+  base::Process process_;
+  int cdp_port_ = 0;  // CDP port (auto-discovered)
+  int mcp_port_ = 0;  // MCP port (auto-discovered)
+  int agent_port_ = 0;  // Agent port (auto-discovered)
+  int extension_port_ = 0;  // Extension port (auto-discovered)
+  bool allow_remote_in_mcp_ = false;  // Whether remote connections allowed in MCP
+  bool is_running_ = false;
+  bool is_restarting_ = false;  // Whether server is currently restarting
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
+  base::WeakPtrFactory<BrowserOSServerManager> weak_factory_{this};
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_MANAGER_H_
