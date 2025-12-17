diff --git a/chrome/browser/browseros/server/browseros_server_manager.cc b/chrome/browser/browseros/server/browseros_server_manager.cc
new file mode 100644
index 0000000000000..114189758b373
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_manager.cc
@@ -0,0 +1,1038 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_server_manager.h"
+
+#include <optional>
+#include <set>
+
+#include "base/command_line.h"
+#include "base/files/file_path.h"
+#include "base/files/file_util.h"
+#include "base/json/json_writer.h"
+#include "base/logging.h"
+#include "base/path_service.h"
+#include "base/process/kill.h"
+#include "base/process/launch.h"
+#include "base/rand_util.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/system/sys_info.h"
+#include "base/task/thread_pool.h"
+#include "base/threading/thread_restrictions.h"
+#include "build/build_config.h"
+#include "chrome/browser/browser_process.h"
+
+#if BUILDFLAG(IS_POSIX)
+#include <signal.h>
+#endif
+
+#include "chrome/browser/browseros/metrics/browseros_metrics_service.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics_service_factory.h"
+#include "chrome/browser/browseros/server/browseros_server_prefs.h"
+#include "chrome/browser/net/system_network_context_manager.h"
+#include "chrome/browser/profiles/profile.h"
+#include "chrome/browser/profiles/profile_manager.h"
+#include "chrome/common/chrome_paths.h"
+#include "components/prefs/pref_change_registrar.h"
+#include "components/prefs/pref_service.h"
+#include "components/version_info/version_info.h"
+#include "content/public/browser/devtools_agent_host.h"
+#include "content/public/browser/devtools_socket_factory.h"
+#include "content/public/browser/storage_partition.h"
+#include "net/base/net_errors.h"
+#include "net/base/port_util.h"
+#include "net/log/net_log_source.h"
+#include "net/socket/tcp_server_socket.h"
+#include "net/traffic_annotation/network_traffic_annotation.h"
+#include "services/network/public/cpp/resource_request.h"
+#include "services/network/public/cpp/simple_url_loader.h"
+#include "services/network/public/mojom/url_loader_factory.mojom.h"
+#include "url/gurl.h"
+
+namespace {
+
+constexpr int kBackLog = 10;
+constexpr base::FilePath::CharType kConfigFileName[] =
+    FILE_PATH_LITERAL("server_config.json");
+
+constexpr base::TimeDelta kHealthCheckInterval = base::Seconds(30);
+constexpr base::TimeDelta kHealthCheckTimeout = base::Seconds(15);
+constexpr base::TimeDelta kProcessCheckInterval = base::Seconds(10);
+
+constexpr int kMaxPortAttempts = 100;
+constexpr int kMaxPort = 65535;
+
+// Holds configuration data gathered on UI thread, passed to background thread
+struct ServerConfig {
+  std::string install_id;
+  std::string browseros_version;
+  std::string chromium_version;
+  bool allow_remote_in_mcp = false;
+};
+
+// Writes the server configuration to a JSON file.
+// Returns the path to the config file on success, empty path on failure.
+base::FilePath WriteConfigJson(const base::FilePath& execution_dir,
+                               const base::FilePath& resources_dir,
+                               uint16_t cdp_port,
+                               uint16_t mcp_port,
+                               uint16_t agent_port,
+                               uint16_t extension_port,
+                               const ServerConfig& server_config) {
+  base::FilePath config_path =
+      execution_dir.Append(kConfigFileName);
+
+  base::Value::Dict config;
+
+  // ports
+  base::Value::Dict ports;
+  ports.Set("cdp", static_cast<int>(cdp_port));
+  ports.Set("http_mcp", static_cast<int>(mcp_port));
+  ports.Set("agent", static_cast<int>(agent_port));
+  ports.Set("extension", static_cast<int>(extension_port));
+  config.Set("ports", std::move(ports));
+
+  // directories
+  base::Value::Dict directories;
+  directories.Set("resources", resources_dir.AsUTF8Unsafe());
+  directories.Set("execution", execution_dir.AsUTF8Unsafe());
+  config.Set("directories", std::move(directories));
+
+  // flags
+  base::Value::Dict flags;
+  flags.Set("allow_remote_in_mcp", server_config.allow_remote_in_mcp);
+  config.Set("flags", std::move(flags));
+
+  // instance
+  base::Value::Dict instance;
+  instance.Set("install_id", server_config.install_id);
+  instance.Set("browseros_version", server_config.browseros_version);
+  instance.Set("chromium_version", server_config.chromium_version);
+  config.Set("instance", std::move(instance));
+
+  std::optional<std::string> json_output = base::WriteJson(config);
+  if (!json_output.has_value()) {
+    LOG(ERROR) << "browseros: Failed to serialize config to JSON";
+    return base::FilePath();
+  }
+
+  if (!base::WriteFile(config_path, json_output.value())) {
+    LOG(ERROR) << "browseros: Failed to write config file: " << config_path;
+    return base::FilePath();
+  }
+
+  LOG(INFO) << "browseros: Wrote config to " << config_path;
+  return config_path;
+}
+
+// Helper function to check for command-line port override.
+// Returns the port value if valid override is found, 0 otherwise.
+int GetPortOverrideFromCommandLine(base::CommandLine* command_line,
+                                    const char* switch_name,
+                                    const char* port_name) {
+  if (!command_line->HasSwitch(switch_name)) {
+    return 0;
+  }
+
+  std::string port_str = command_line->GetSwitchValueASCII(switch_name);
+  int port = 0;
+
+  if (!base::StringToInt(port_str, &port) || !net::IsPortValid(port) ||
+      port <= 0) {
+    LOG(WARNING) << "browseros: Invalid " << port_name
+                 << " specified on command line: " << port_str
+                 << " (must be 1-65535)";
+    return 0;
+  }
+
+  // Warn about problematic ports but respect explicit user intent
+  if (net::IsWellKnownPort(port)) {
+    LOG(WARNING) << "browseros: " << port_name << " " << port
+                 << " is well-known (0-1023) and may require elevated "
+                    "privileges";
+  }
+  if (!net::IsPortAllowedForScheme(port, "http")) {
+    LOG(WARNING) << "browseros: " << port_name << " " << port
+                 << " is restricted by Chromium (may interfere with system "
+                    "services)";
+  }
+
+  LOG(INFO) << "browseros: " << port_name
+            << " overridden via command line: " << port;
+  return port;
+}
+
+// Launches the BrowserOS server process on a background thread.
+// This function performs blocking I/O operations (PathExists, WriteConfigToml,
+// LaunchProcess).
+base::Process LaunchProcessOnBackgroundThread(
+    const base::FilePath& exe_path,
+    const base::FilePath& resources_dir,
+    const base::FilePath& execution_dir,
+    uint16_t cdp_port,
+    uint16_t mcp_port,
+    uint16_t agent_port,
+    uint16_t extension_port,
+    const ServerConfig& server_config) {
+  // Check if executable exists (blocking I/O)
+  if (!base::PathExists(exe_path)) {
+    LOG(ERROR) << "browseros: BrowserOS server executable not found at: "
+               << exe_path;
+    return base::Process();
+  }
+
+  if (execution_dir.empty()) {
+    LOG(ERROR) << "browseros: Execution directory path is empty";
+    return base::Process();
+  }
+
+  // Ensure execution directory exists (blocking I/O)
+  if (!base::CreateDirectory(execution_dir)) {
+    LOG(ERROR) << "browseros: Failed to create execution directory at: "
+               << execution_dir;
+    return base::Process();
+  }
+
+  // Write configuration to JSON file
+  base::FilePath config_path = WriteConfigJson(
+      execution_dir, resources_dir, cdp_port, mcp_port, agent_port,
+      extension_port, server_config);
+  if (config_path.empty()) {
+    LOG(ERROR) << "browseros: Failed to write config file, aborting launch";
+    return base::Process();
+  }
+
+  // Build command line with --config flag and explicit port args
+  // Ports are passed via CLI to avoid config file read race conditions
+  // CLI takes precedence over config file in the server's merge logic
+  base::CommandLine cmd(exe_path);
+  cmd.AppendSwitchPath("config", config_path);
+  cmd.AppendSwitchASCII("cdp-port", base::NumberToString(cdp_port));
+  cmd.AppendSwitchASCII("http-mcp-port", base::NumberToString(mcp_port));
+  cmd.AppendSwitchASCII("agent-port", base::NumberToString(agent_port));
+  cmd.AppendSwitchASCII("extension-port", base::NumberToString(extension_port));
+
+  // Set up launch options
+  base::LaunchOptions options;
+#if BUILDFLAG(IS_WIN)
+  options.start_hidden = true;
+#endif
+
+  // Launch the process (blocking I/O)
+  return base::LaunchProcess(cmd, options);
+}
+
+// Factory for creating TCP server sockets for CDP
+class CDPServerSocketFactory : public content::DevToolsSocketFactory {
+ public:
+  explicit CDPServerSocketFactory(uint16_t port) : port_(port) {}
+
+  CDPServerSocketFactory(const CDPServerSocketFactory&) = delete;
+  CDPServerSocketFactory& operator=(const CDPServerSocketFactory&) = delete;
+
+ private:
+  std::unique_ptr<net::ServerSocket> CreateLocalHostServerSocket(int port) {
+    std::unique_ptr<net::ServerSocket> socket(
+        new net::TCPServerSocket(nullptr, net::NetLogSource()));
+    if (socket->ListenWithAddressAndPort("127.0.0.1", port, kBackLog) ==
+        net::OK) {
+      return socket;
+    }
+    if (socket->ListenWithAddressAndPort("::1", port, kBackLog) == net::OK) {
+      return socket;
+    }
+    return nullptr;
+  }
+
+  // content::DevToolsSocketFactory implementation
+  std::unique_ptr<net::ServerSocket> CreateForHttpServer() override {
+    return CreateLocalHostServerSocket(port_);
+  }
+
+  std::unique_ptr<net::ServerSocket> CreateForTethering(
+      std::string* name) override {
+    return nullptr;  // Tethering not needed for BrowserOS
+  }
+
+  uint16_t port_;
+};
+
+}  // namespace
+
+namespace browseros {
+
+// static
+BrowserOSServerManager* BrowserOSServerManager::GetInstance() {
+  static base::NoDestructor<BrowserOSServerManager> instance;
+  return instance.get();
+}
+
+BrowserOSServerManager::BrowserOSServerManager() = default;
+
+BrowserOSServerManager::~BrowserOSServerManager() {
+  Shutdown();
+}
+
+bool BrowserOSServerManager::AcquireLock() {
+  // Allow blocking for lock file operations (short-duration I/O)
+  base::ScopedAllowBlocking allow_blocking;
+
+  base::FilePath exec_dir = GetBrowserOSExecutionDir();
+  if (exec_dir.empty()) {
+    LOG(ERROR) << "browseros: Failed to resolve execution directory for lock";
+    return false;
+  }
+
+  base::FilePath lock_path = exec_dir.Append(FILE_PATH_LITERAL("server.lock"));
+
+  lock_file_ = base::File(lock_path,
+                          base::File::FLAG_OPEN_ALWAYS |
+                          base::File::FLAG_READ |
+                          base::File::FLAG_WRITE);
+
+  if (!lock_file_.IsValid()) {
+    LOG(ERROR) << "browseros: Failed to open lock file: " << lock_path;
+    return false;
+  }
+
+  base::File::Error lock_error =
+      lock_file_.Lock(base::File::LockMode::kExclusive);
+  if (lock_error != base::File::FILE_OK) {
+    LOG(INFO) << "browseros: Server already running in another Chrome process "
+              << "(lock file: " << lock_path << ")";
+    lock_file_.Close();
+    return false;
+  }
+
+  LOG(INFO) << "browseros: Acquired exclusive lock on " << lock_path;
+  return true;
+}
+
+void BrowserOSServerManager::InitializePortsAndPrefs() {
+  base::CommandLine* command_line = base::CommandLine::ForCurrentProcess();
+  PrefService* prefs = g_browser_process->local_state();
+
+  // Read from prefs or use defaults
+  if (!prefs) {
+    cdp_port_ = browseros_server::kDefaultCDPPort;
+    mcp_port_ = browseros_server::kDefaultMCPPort;
+    agent_port_ = browseros_server::kDefaultAgentPort;
+    extension_port_ = browseros_server::kDefaultExtensionPort;
+    allow_remote_in_mcp_ = false;
+  } else {
+    cdp_port_ = prefs->GetInteger(browseros_server::kCDPServerPort);
+    if (cdp_port_ <= 0) {
+      cdp_port_ = browseros_server::kDefaultCDPPort;
+    }
+
+    mcp_port_ = prefs->GetInteger(browseros_server::kMCPServerPort);
+    if (mcp_port_ <= 0) {
+      mcp_port_ = browseros_server::kDefaultMCPPort;
+    }
+
+    agent_port_ = prefs->GetInteger(browseros_server::kAgentServerPort);
+    if (agent_port_ <= 0) {
+      agent_port_ = browseros_server::kDefaultAgentPort;
+    }
+
+    extension_port_ = prefs->GetInteger(browseros_server::kExtensionServerPort);
+    if (extension_port_ <= 0) {
+      extension_port_ = browseros_server::kDefaultExtensionPort;
+    }
+
+    allow_remote_in_mcp_ = prefs->GetBoolean(browseros_server::kAllowRemoteInMCP);
+
+    // Set up pref change observers
+    if (!pref_change_registrar_) {
+      pref_change_registrar_ = std::make_unique<PrefChangeRegistrar>();
+      pref_change_registrar_->Init(prefs);
+      pref_change_registrar_->Add(
+          browseros_server::kAllowRemoteInMCP,
+          base::BindRepeating(
+              &BrowserOSServerManager::OnAllowRemoteInMCPChanged,
+              base::Unretained(this)));
+      pref_change_registrar_->Add(
+          browseros_server::kRestartServerRequested,
+          base::BindRepeating(
+              &BrowserOSServerManager::OnRestartServerRequestedChanged,
+              base::Unretained(this)));
+    }
+  }
+
+  // Find available ports, tracking assigned ports to prevent collisions
+  std::set<int> assigned_ports;
+
+  cdp_port_ = FindAvailablePort(cdp_port_, assigned_ports);
+  assigned_ports.insert(cdp_port_);
+
+  mcp_port_ = FindAvailablePort(mcp_port_, assigned_ports);
+  assigned_ports.insert(mcp_port_);
+
+  agent_port_ = FindAvailablePort(agent_port_, assigned_ports);
+  assigned_ports.insert(agent_port_);
+
+  extension_port_ = FindAvailablePort(extension_port_, assigned_ports);
+
+  // Apply command-line overrides (internal testing only)
+  int cdp_override = GetPortOverrideFromCommandLine(
+      command_line, "browseros-cdp-port", "CDP port");
+  if (cdp_override > 0) {
+    cdp_port_ = cdp_override;
+  }
+
+  int mcp_override = GetPortOverrideFromCommandLine(
+      command_line, "browseros-mcp-port", "MCP port");
+  if (mcp_override > 0) {
+    mcp_port_ = mcp_override;
+  }
+
+  int agent_override = GetPortOverrideFromCommandLine(
+      command_line, "browseros-agent-port", "Agent port");
+  if (agent_override > 0) {
+    agent_port_ = agent_override;
+  }
+
+  int extension_override = GetPortOverrideFromCommandLine(
+      command_line, "browseros-extension-port", "Extension port");
+  if (extension_override > 0) {
+    extension_port_ = extension_override;
+  }
+
+  LOG(INFO) << "browseros: Final ports - CDP: " << cdp_port_
+            << ", MCP: " << mcp_port_ << ", Agent: " << agent_port_
+            << ", Extension: " << extension_port_;
+}
+
+void BrowserOSServerManager::SavePortsToPrefs() {
+  PrefService* prefs = g_browser_process->local_state();
+  if (!prefs) {
+    LOG(WARNING) << "browseros: SavePortsToPrefs - no prefs available, skipping save";
+    return;
+  }
+
+  prefs->SetInteger(browseros_server::kCDPServerPort, cdp_port_);
+  prefs->SetInteger(browseros_server::kMCPServerPort, mcp_port_);
+  prefs->SetInteger(browseros_server::kAgentServerPort, agent_port_);
+  prefs->SetInteger(browseros_server::kExtensionServerPort, extension_port_);
+
+  LOG(INFO) << "browseros: Saving to prefs - CDP: " << cdp_port_
+            << ", MCP: " << mcp_port_ << ", Agent: " << agent_port_
+            << ", Extension: " << extension_port_;
+}
+
+void BrowserOSServerManager::Start() {
+  if (is_running_) {
+    LOG(INFO) << "browseros: BrowserOS server already running";
+    return;
+  }
+
+  base::CommandLine* command_line = base::CommandLine::ForCurrentProcess();
+  // Initialize and finalize ports, even with browseros-server disabled
+  // we want to update the prefs from CLI
+  InitializePortsAndPrefs();
+  SavePortsToPrefs();
+
+  if (command_line->HasSwitch("disable-browseros-server")) {
+    LOG(INFO) << "browseros: BrowserOS server disabled via command line";
+    return;
+  }
+
+  // Try to acquire system-wide lock
+  if (!AcquireLock()) {
+    return;  // Another Chrome process already owns the server
+  }
+
+  LOG(INFO) << "browseros: Starting BrowserOS server";
+
+  // Start servers and process
+  // Note: monitoring timers are started in OnProcessLaunched() after successful launch
+  StartCDPServer();
+  LaunchBrowserOSProcess();
+}
+
+void BrowserOSServerManager::Stop() {
+  if (!is_running_) {
+    return;
+  }
+
+  LOG(INFO) << "browseros: Stopping BrowserOS server";
+  health_check_timer_.Stop();
+  process_check_timer_.Stop();
+
+  // Use wait=false for shutdown - just send kill signal, don't block UI thread
+  TerminateBrowserOSProcess(/*wait=*/false);
+
+  // Release lock
+  if (lock_file_.IsValid()) {
+    lock_file_.Unlock();
+    lock_file_.Close();
+    LOG(INFO) << "browseros: Released lock file";
+  }
+}
+
+bool BrowserOSServerManager::IsRunning() const {
+  return is_running_ && process_.IsValid();
+}
+
+void BrowserOSServerManager::Shutdown() {
+  Stop();
+}
+
+void BrowserOSServerManager::StartCDPServer() {
+  LOG(INFO) << "browseros: Starting CDP server on port " << cdp_port_;
+
+  content::DevToolsAgentHost::StartRemoteDebuggingServer(
+      std::make_unique<CDPServerSocketFactory>(cdp_port_),
+      base::FilePath(),
+      base::FilePath());
+
+  LOG(INFO) << "browseros: CDP WebSocket server started at ws://127.0.0.1:"
+            << cdp_port_;
+  LOG(INFO) << "browseros: MCP server port: " << mcp_port_
+            << " (allow_remote: "
+            << (allow_remote_in_mcp_ ? "true" : "false") << ")";
+  LOG(INFO) << "browseros: Agent server port: " << agent_port_;
+  LOG(INFO) << "browseros: Extension server port: " << extension_port_;
+}
+
+void BrowserOSServerManager::StopCDPServer() {
+  if (cdp_port_ == 0) {
+    return;
+  }
+
+  LOG(INFO) << "browseros: Stopping CDP server";
+  content::DevToolsAgentHost::StopRemoteDebuggingServer();
+  cdp_port_ = 0;
+}
+
+void BrowserOSServerManager::LaunchBrowserOSProcess() {
+  base::FilePath exe_path = GetBrowserOSServerExecutablePath();
+  base::FilePath resources_dir = GetBrowserOSServerResourcesPath();
+  base::FilePath execution_dir = GetBrowserOSExecutionDir();
+  if (execution_dir.empty()) {
+    LOG(ERROR) << "browseros: Failed to resolve execution directory";
+    return;
+  }
+
+  LOG(INFO) << "browseros: Launching server - binary: " << exe_path;
+  LOG(INFO) << "browseros: Launching server - resources: " << resources_dir;
+  LOG(INFO) << "browseros: Launching server - execution dir: " << execution_dir;
+
+  // Capture values to pass to background thread
+  uint16_t cdp_port = cdp_port_;
+  uint16_t mcp_port = mcp_port_;
+  uint16_t agent_port = agent_port_;
+  uint16_t extension_port = extension_port_;
+
+  // Gather server config on UI thread
+  ServerConfig server_config;
+  server_config.browseros_version =
+      std::string(version_info::GetBrowserOSVersionNumber());
+  server_config.chromium_version =
+      std::string(version_info::GetVersionNumber());
+  server_config.allow_remote_in_mcp = allow_remote_in_mcp_;
+
+  // Get install_id from BrowserOSMetricsService if available
+  ProfileManager* profile_manager = g_browser_process->profile_manager();
+  if (profile_manager) {
+    Profile* profile = profile_manager->GetLastUsedProfileIfLoaded();
+    if (profile && !profile->IsOffTheRecord()) {
+      browseros_metrics::BrowserOSMetricsService* metrics_service =
+          browseros_metrics::BrowserOSMetricsServiceFactory::GetForBrowserContext(
+              profile);
+      if (metrics_service) {
+        server_config.install_id = metrics_service->GetInstallId();
+      }
+    }
+  }
+
+  // Post blocking work to background thread, get result back on UI thread
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_BLOCKING},
+      base::BindOnce(&LaunchProcessOnBackgroundThread, exe_path, resources_dir,
+                     execution_dir, cdp_port, mcp_port, agent_port,
+                     extension_port, server_config),
+      base::BindOnce(&BrowserOSServerManager::OnProcessLaunched,
+                     weak_factory_.GetWeakPtr()));
+}
+
+void BrowserOSServerManager::OnProcessLaunched(base::Process process) {
+  if (!process.IsValid()) {
+    LOG(ERROR) << "browseros: Failed to launch BrowserOS server";
+    // Don't stop CDP server - it's independent and may be used by other things
+    // Leave system in degraded state (CDP up, no browseros_server) rather than
+    // completely broken state (no CDP, no server)
+    is_restarting_ = false;
+    return;
+  }
+
+  process_ = std::move(process);
+  is_running_ = true;
+
+  LOG(INFO) << "browseros: BrowserOS server started with PID: " << process_.Pid();
+  LOG(INFO) << "browseros: CDP port: " << cdp_port_;
+  LOG(INFO) << "browseros: MCP port: " << mcp_port_;
+  LOG(INFO) << "browseros: Agent port: " << agent_port_;
+  LOG(INFO) << "browseros: Extension port: " << extension_port_;
+
+  // Start/restart monitoring timers
+  health_check_timer_.Start(FROM_HERE, kHealthCheckInterval, this,
+                            &BrowserOSServerManager::CheckServerHealth);
+  process_check_timer_.Start(FROM_HERE, kProcessCheckInterval, this,
+                             &BrowserOSServerManager::CheckProcessStatus);
+
+  // Reset restart flag and pref after successful launch
+  if (is_restarting_) {
+    is_restarting_ = false;
+    PrefService* prefs = g_browser_process->local_state();
+    if (prefs && prefs->GetBoolean(browseros_server::kRestartServerRequested)) {
+      prefs->SetBoolean(browseros_server::kRestartServerRequested, false);
+      LOG(INFO) << "browseros: Restart completed, reset restart_requested pref";
+    }
+  }
+}
+
+void BrowserOSServerManager::TerminateBrowserOSProcess(bool wait) {
+  if (!process_.IsValid()) {
+    return;
+  }
+
+  LOG(INFO) << "browseros: Terminating BrowserOS server process (PID: "
+            << process_.Pid() << ", wait: " << (wait ? "true" : "false") << ")";
+
+#if BUILDFLAG(IS_POSIX)
+  base::ProcessId pid = process_.Pid();
+  if (kill(pid, SIGKILL) != 0) {
+    PLOG(ERROR) << "browseros: Failed to send SIGKILL to PID " << pid;
+  } else if (wait) {
+    // Blocking wait - must be called from background thread
+    base::ScopedAllowBaseSyncPrimitives allow_sync;
+    base::ScopedAllowBlocking allow_blocking;
+    int exit_code = 0;
+    if (process_.WaitForExit(&exit_code)) {
+      LOG(INFO) << "browseros: Process killed successfully";
+    } else {
+      LOG(WARNING) << "browseros: WaitForExit failed";
+    }
+  } else {
+    LOG(INFO) << "browseros: SIGKILL sent (not waiting for exit)";
+  }
+#else
+  // Windows: Terminate with wait parameter
+  bool terminated = process_.Terminate(0, wait);
+  if (terminated) {
+    LOG(INFO) << "browseros: Process terminated successfully";
+  } else {
+    LOG(ERROR) << "browseros: Failed to terminate process";
+  }
+#endif
+
+  is_running_ = false;
+}
+
+void BrowserOSServerManager::OnProcessExited(int exit_code) {
+  LOG(INFO) << "browseros: BrowserOS server exited with code: " << exit_code;
+  is_running_ = false;
+
+  // Stop timers during restart to prevent races
+  health_check_timer_.Stop();
+  process_check_timer_.Stop();
+
+  // Prevent concurrent restarts (e.g., if RestartBrowserOSProcess is in progress)
+  if (is_restarting_) {
+    LOG(INFO) << "browseros: Restart already in progress, skipping";
+    return;
+  }
+  is_restarting_ = true;
+
+  // Always restart - we want the server running
+  // Don't call Start() - we already hold the lock and CDP server is running
+  LOG(WARNING) << "browseros: BrowserOS server exited, restarting process...";
+
+  // Capture current ports for background thread
+  int cdp = cdp_port_;
+  int mcp = mcp_port_;
+  int agent = agent_port_;
+  int extension = extension_port_;
+
+  // Revalidate ports on background thread, then launch on UI thread
+  // Process is already dead, no need to terminate
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_BLOCKING},
+      base::BindOnce(&BrowserOSServerManager::RevalidatePorts,
+                     base::Unretained(this), cdp, mcp, agent, extension),
+      base::BindOnce(&BrowserOSServerManager::OnPortsRevalidated,
+                     weak_factory_.GetWeakPtr()));
+}
+
+void BrowserOSServerManager::CheckServerHealth() {
+  if (!is_running_) {
+    return;
+  }
+
+  // Build health check URL
+  GURL health_url("http://127.0.0.1:" + base::NumberToString(mcp_port_) + "/health");
+
+  // Create network traffic annotation
+  net::NetworkTrafficAnnotationTag traffic_annotation =
+      net::DefineNetworkTrafficAnnotation("browseros_health_check", R"(
+        semantics {
+          sender: "BrowserOS Server Manager"
+          description:
+            "Checks if the BrowserOS MCP server is healthy by querying its "
+            "/health endpoint."
+          trigger: "Periodic health check every 60 seconds while server is running."
+          data: "No user data sent, just an HTTP GET request."
+          destination: LOCAL
+        }
+        policy {
+          cookies_allowed: NO
+          setting: "This feature cannot be disabled by settings."
+          policy_exception_justification:
+            "Internal health check for BrowserOS server functionality."
+        })");
+
+  // Create resource request
+  auto resource_request = std::make_unique<network::ResourceRequest>();
+  resource_request->url = health_url;
+  resource_request->method = "GET";
+  resource_request->credentials_mode = network::mojom::CredentialsMode::kOmit;
+
+  auto url_loader = network::SimpleURLLoader::Create(
+      std::move(resource_request), traffic_annotation);
+  url_loader->SetTimeoutDuration(kHealthCheckTimeout);
+
+  // Get URL loader factory from default storage partition
+  auto* url_loader_factory =
+      g_browser_process->system_network_context_manager()
+          ->GetURLLoaderFactory();
+
+  // Keep a raw pointer for the callback
+  auto* url_loader_ptr = url_loader.get();
+
+  // Download response
+  url_loader_ptr->DownloadHeadersOnly(
+      url_loader_factory,
+      base::BindOnce(&BrowserOSServerManager::OnHealthCheckComplete,
+                     weak_factory_.GetWeakPtr(), std::move(url_loader)));
+}
+
+void BrowserOSServerManager::CheckProcessStatus() {
+  if (!is_running_ || !process_.IsValid()) {
+    return;
+  }
+
+  int exit_code = 0;
+  bool exited = process_.WaitForExitWithTimeout(base::TimeDelta(), &exit_code);
+  LOG(INFO) << "browseros: CheckProcessStatus PID: " << process_.Pid()
+            << ", WaitForExitWithTimeout returned: " << exited
+            << ", exit_code: " << exit_code;
+
+  if (exited) {
+    OnProcessExited(exit_code);
+  }
+}
+
+void BrowserOSServerManager::OnHealthCheckComplete(
+    std::unique_ptr<network::SimpleURLLoader> url_loader,
+    scoped_refptr<net::HttpResponseHeaders> headers) {
+  if (!is_running_) {
+    return;
+  }
+
+  // Check if we got a valid response
+  int response_code = 0;
+  if (headers) {
+    response_code = headers->response_code();
+  }
+
+  if (response_code == 200) {
+    LOG(INFO) << "browseros: Health check passed";
+    return;
+  }
+
+  // Health check failed
+  int net_error = url_loader->NetError();
+  LOG(WARNING) << "browseros: Health check failed - HTTP " << response_code
+               << ", net error: " << net::ErrorToString(net_error)
+               << ", restarting BrowserOS server process...";
+
+  RestartBrowserOSProcess();
+}
+
+void BrowserOSServerManager::RestartBrowserOSProcess() {
+  LOG(INFO) << "browseros: Restarting BrowserOS server process";
+
+  // Prevent multiple concurrent restarts
+  if (is_restarting_) {
+    LOG(INFO) << "browseros: Restart already in progress, ignoring";
+    return;
+  }
+  is_restarting_ = true;
+
+  // Stop all timers during restart to prevent races
+  health_check_timer_.Stop();
+  process_check_timer_.Stop();
+
+  // Capture current ports for background thread
+  int cdp = cdp_port_;
+  int mcp = mcp_port_;
+  int agent = agent_port_;
+  int extension = extension_port_;
+
+  // Kill process on background thread, revalidate ports, then launch on UI thread
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_BLOCKING},
+      base::BindOnce(
+          [](BrowserOSServerManager* manager, int cdp, int mcp, int agent,
+             int extension) -> RevalidatedPorts {
+            manager->TerminateBrowserOSProcess(/*wait=*/true);
+            return manager->RevalidatePorts(cdp, mcp, agent, extension);
+          },
+          base::Unretained(this), cdp, mcp, agent, extension),
+      base::BindOnce(&BrowserOSServerManager::OnPortsRevalidated,
+                     weak_factory_.GetWeakPtr()));
+}
+
+BrowserOSServerManager::RevalidatedPorts BrowserOSServerManager::RevalidatePorts(
+    int cdp_port,
+    int current_mcp,
+    int current_agent,
+    int current_extension) {
+  // CDP port is excluded - it's still bound by Chrome's DevTools server
+  std::set<int> excluded_ports;
+  excluded_ports.insert(cdp_port);
+
+  RevalidatedPorts result;
+  result.mcp_port = FindAvailablePort(current_mcp, excluded_ports);
+  excluded_ports.insert(result.mcp_port);
+
+  result.agent_port = FindAvailablePort(current_agent, excluded_ports);
+  excluded_ports.insert(result.agent_port);
+
+  result.extension_port = FindAvailablePort(current_extension, excluded_ports);
+
+  return result;
+}
+
+void BrowserOSServerManager::OnPortsRevalidated(RevalidatedPorts ports) {
+  bool ports_changed = (ports.mcp_port != mcp_port_) ||
+                       (ports.agent_port != agent_port_) ||
+                       (ports.extension_port != extension_port_);
+
+  if (ports_changed) {
+    LOG(INFO) << "browseros: Ports changed during revalidation - "
+              << "MCP: " << mcp_port_ << " -> " << ports.mcp_port
+              << ", Agent: " << agent_port_ << " -> " << ports.agent_port
+              << ", Extension: " << extension_port_ << " -> "
+              << ports.extension_port;
+
+    mcp_port_ = ports.mcp_port;
+    agent_port_ = ports.agent_port;
+    extension_port_ = ports.extension_port;
+    SavePortsToPrefs();
+  }
+
+  // Note: is_restarting_ is cleared in OnProcessLaunched() after launch completes
+  LaunchBrowserOSProcess();
+}
+
+void BrowserOSServerManager::OnAllowRemoteInMCPChanged() {
+  if (!is_running_) {
+    return;
+  }
+
+  PrefService* prefs = g_browser_process->local_state();
+  if (!prefs) {
+    return;
+  }
+
+  bool new_value = prefs->GetBoolean(browseros_server::kAllowRemoteInMCP);
+
+  if (new_value != allow_remote_in_mcp_) {
+    LOG(INFO) << "browseros: allow_remote_in_mcp preference changed from "
+              << (allow_remote_in_mcp_ ? "true" : "false") << " to "
+              << (new_value ? "true" : "false")
+              << ", restarting server...";
+
+    allow_remote_in_mcp_ = new_value;
+
+    // Restart server to apply new config
+    RestartBrowserOSProcess();
+  }
+}
+
+void BrowserOSServerManager::OnRestartServerRequestedChanged() {
+  PrefService* prefs = g_browser_process->local_state();
+  if (!prefs) {
+    return;
+  }
+
+  bool restart_requested = prefs->GetBoolean(browseros_server::kRestartServerRequested);
+
+  // Only process if pref is set to true
+  if (!restart_requested) {
+    return;
+  }
+
+  LOG(INFO) << "browseros: Server restart requested via preference";
+  RestartBrowserOSProcess();
+}
+
+int BrowserOSServerManager::FindAvailablePort(
+    int starting_port,
+    const std::set<int>& excluded_ports) {
+  LOG(INFO) << "browseros: Finding port starting from " << starting_port;
+
+  for (int i = 0; i < kMaxPortAttempts; i++) {
+    int port_to_try = starting_port + i;
+
+    if (port_to_try > kMaxPort) {
+      break;
+    }
+
+    // Skip ports already assigned to other BrowserOS services
+    if (excluded_ports.count(port_to_try) > 0) {
+      continue;
+    }
+
+    if (IsPortAvailable(port_to_try)) {
+      if (port_to_try != starting_port) {
+        LOG(INFO) << "browseros: Port " << starting_port
+                  << " was in use or excluded, using " << port_to_try
+                  << " instead";
+      } else {
+        LOG(INFO) << "browseros: Using port " << port_to_try;
+      }
+      return port_to_try;
+    }
+  }
+
+  LOG(WARNING) << "browseros: Could not find available port after "
+               << kMaxPortAttempts
+               << " attempts, using " << starting_port << " anyway";
+  return starting_port;
+}
+
+bool BrowserOSServerManager::IsPortAvailable(int port) {
+  // Check port is in valid range
+  if (!net::IsPortValid(port) || port == 0) {
+    return false;
+  }
+
+  // Avoid well-known ports (0-1023, require elevated privileges)
+  if (net::IsWellKnownPort(port)) {
+    return false;
+  }
+
+  // Avoid restricted ports (could interfere with system services)
+  if (!net::IsPortAllowedForScheme(port, "http")) {
+    return false;
+  }
+
+  // Try to bind to both IPv4 and IPv6 localhost
+  // If EITHER is in use, the port is NOT available
+  std::unique_ptr<net::TCPServerSocket> socket(
+      new net::TCPServerSocket(nullptr, net::NetLogSource()));
+
+  // Try binding to IPv4 localhost
+  int result = socket->ListenWithAddressAndPort("127.0.0.1", port, 1);
+  if (result != net::OK) {
+    return false;  // IPv4 port is in use
+  }
+
+  // Try binding to IPv6 localhost
+  std::unique_ptr<net::TCPServerSocket> socket6(
+      new net::TCPServerSocket(nullptr, net::NetLogSource()));
+  int result6 = socket6->ListenWithAddressAndPort("::1", port, 1);
+  if (result6 != net::OK) {
+    return false;  // IPv6 port is in use
+  }
+
+  return true;
+}
+
+base::FilePath BrowserOSServerManager::GetBrowserOSServerResourcesPath() const {
+  // Check for command-line override first
+  base::CommandLine* command_line = base::CommandLine::ForCurrentProcess();
+  if (command_line->HasSwitch("browseros-server-resources-dir")) {
+    base::FilePath custom_path =
+        command_line->GetSwitchValuePath("browseros-server-resources-dir");
+    LOG(INFO) << "browseros: Using custom resources dir from command line: "
+              << custom_path;
+    return custom_path;
+  }
+
+  base::FilePath exe_dir;
+
+#if BUILDFLAG(IS_MAC)
+  // On macOS, the binary will be in the app bundle
+  if (!base::PathService::Get(base::DIR_EXE, &exe_dir)) {
+    LOG(ERROR) << "browseros: Failed to get executable directory";
+    return base::FilePath();
+  }
+
+  // Navigate to Resources folder in the app bundle
+  // Chrome.app/Contents/MacOS -> Chrome.app/Contents/Resources
+  exe_dir = exe_dir.DirName().Append("Resources");
+
+#elif BUILDFLAG(IS_WIN)
+  // On Windows, installer places BrowserOS Server under the versioned directory
+  if (!base::PathService::Get(base::DIR_EXE, &exe_dir)) {
+    LOG(ERROR) << "browseros: Failed to get executable directory";
+    return base::FilePath();
+  }
+  // Append version directory (chrome.release places BrowserOSServer under versioned dir)
+  exe_dir = exe_dir.AppendASCII(version_info::GetVersionNumber());
+
+#elif BUILDFLAG(IS_LINUX)
+  // On Linux, binary is in the same directory as chrome
+  if (!base::PathService::Get(base::DIR_EXE, &exe_dir)) {
+    LOG(ERROR) << "browseros: Failed to get executable directory";
+    return base::FilePath();
+  }
+#endif
+
+  // Return path to resources directory
+  return exe_dir.Append(FILE_PATH_LITERAL("BrowserOSServer"))
+      .Append(FILE_PATH_LITERAL("default"))
+      .Append(FILE_PATH_LITERAL("resources"));
+}
+
+base::FilePath BrowserOSServerManager::GetBrowserOSExecutionDir() const {
+  base::FilePath user_data_dir;
+  if (!base::PathService::Get(chrome::DIR_USER_DATA, &user_data_dir)) {
+    LOG(ERROR) << "browseros: Failed to resolve DIR_USER_DATA path";
+    return base::FilePath();
+  }
+
+  base::FilePath exec_dir = user_data_dir.Append(FILE_PATH_LITERAL(".browseros"));
+
+  // Ensure directory exists before returning
+  base::ScopedAllowBlocking allow_blocking;
+  if (!base::PathExists(exec_dir)) {
+    if (!base::CreateDirectory(exec_dir)) {
+      LOG(ERROR) << "browseros: Failed to create execution directory: " << exec_dir;
+      return base::FilePath();
+    }
+  }
+
+  LOG(INFO) << "browseros: Using execution directory: " << exec_dir;
+  return exec_dir;
+}
+
+base::FilePath BrowserOSServerManager::GetBrowserOSServerExecutablePath() const {
+  base::FilePath browseros_exe =
+      GetBrowserOSServerResourcesPath()
+          .Append(FILE_PATH_LITERAL("bin"))
+          .Append(FILE_PATH_LITERAL("browseros_server"));
+
+#if BUILDFLAG(IS_WIN)
+  browseros_exe = browseros_exe.AddExtension(FILE_PATH_LITERAL(".exe"));
+#endif
+
+  return browseros_exe;
+}
+
+}  // namespace browseros
