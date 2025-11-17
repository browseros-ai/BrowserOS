diff --git a/chrome/browser/blockbrowser_server/blockbrowser_server_manager.h b/chrome/browser/blockbrowser_server/blockbrowser_server_manager.h
new file mode 100644
index 0000000000000..d991d965c4913
--- /dev/null
+++ b/chrome/browser/blockbrowser_server/blockbrowser_server_manager.h
@@ -0,0 +1,138 @@
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
+// BlockBrowser: Manages the lifecycle of the BlockBrowser server process (singleton)
+// This manager:
+// 1. Starts Chromium's CDP WebSocket server (port 9222+, auto-discovered)
+// 2. Launches the bundled BlockBrowser server binary with CDP and MCP ports
+// 3. Monitors MCP server health via HTTP /health endpoint and auto-restarts
+class BlockBrowserServerManager {
+ public:
+  static BlockBrowserServerManager* GetInstance();
+
+  BlockBrowserServerManager(const BlockBrowserServerManager&) = delete;
+  BlockBrowserServerManager& operator=(const BlockBrowserServerManager&) = delete;
+
+  // Starts the BlockBrowser server if not already running
+  // This will:
+  // 1. Find available CDP port (starting from 9222 or saved pref)
+  // 2. Start CDP WebSocket server on discovered port
+  // 3. Find available MCP port (starting from 9223 or saved pref)
+  // 4. Launch blockbrowser_server binary with discovered ports
+  void Start();
+
+  // Stops the BlockBrowser server
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
+  // Returns whether MCP server is enabled
+  bool IsMCPEnabled() const { return mcp_enabled_; }
+
+  // Called when browser is shutting down
+  void Shutdown();
+
+ private:
+  friend base::NoDestructor<BlockBrowserServerManager>;
+
+  BlockBrowserServerManager();
+  ~BlockBrowserServerManager();
+
+  bool AcquireLock();
+  void InitializePortsAndPrefs();
+  void SavePortsToPrefs();
+  void StartCDPServer();
+  void StopCDPServer();
+  void LaunchBlockBrowserProcess();
+  void OnProcessLaunched(base::Process process);
+  void TerminateBlockBrowserProcess();
+  void RestartBlockBrowserProcess();
+  void OnProcessExited(int exit_code);
+  void CheckServerHealth();
+  void OnHealthCheckComplete(
+      std::unique_ptr<network::SimpleURLLoader> url_loader,
+      scoped_refptr<net::HttpResponseHeaders> headers);
+  void OnMCPEnabledChanged();
+  void OnRestartServerRequestedChanged();
+  void SendMCPControlRequest(bool enabled);
+  void OnMCPControlRequestComplete(
+      bool requested_state,
+      std::unique_ptr<network::SimpleURLLoader> url_loader,
+      scoped_refptr<net::HttpResponseHeaders> headers);
+  void SendInitRequest();
+  void OnInitRequestComplete(
+      std::unique_ptr<network::SimpleURLLoader> url_loader,
+      scoped_refptr<net::HttpResponseHeaders> headers);
+  void CheckProcessStatus();
+
+  base::FilePath GetBlockBrowserServerResourcesPath() const;
+  base::FilePath GetBlockBrowserExecutionDir() const;
+  base::FilePath GetBlockBrowserServerExecutablePath() const;
+  int FindAvailablePort(int starting_port);
+  bool IsPortAvailable(int port);
+
+  base::File lock_file_;  // System-wide lock to ensure single instance
+  base::Process process_;
+  int cdp_port_ = 0;  // CDP port (auto-discovered)
+  int mcp_port_ = 0;  // MCP port (auto-discovered)
+  int agent_port_ = 0;  // Agent port (auto-discovered)
+  int extension_port_ = 0;  // Extension port (auto-discovered)
+  bool mcp_enabled_ = true;  // Whether MCP server is enabled
+  bool is_running_ = false;
+  bool is_restarting_ = false;  // Whether server is currently restarting
+  bool init_request_sent_ = false;  // Whether /init request has been sent
+
+  // Timer for health checks
+  base::RepeatingTimer health_check_timer_;
+
+  // Timer for process status checks
+  base::RepeatingTimer process_check_timer_;
+
+  // Preference change registrar for monitoring MCP enabled changes
+  std::unique_ptr<PrefChangeRegistrar> pref_change_registrar_;
+
+  base::WeakPtrFactory<BlockBrowserServerManager> weak_factory_{this};
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_MANAGER_H_
