diff --git a/chrome/browser/browseros/server/browseros_server_manager.cc b/chrome/browser/browseros/server/browseros_server_manager.cc
new file mode 100644
index 0000000000000..9908cbc8930f9
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_manager.cc
@@ -0,0 +1,998 @@
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
+#include "base/logging.h"
+#include "base/path_service.h"
+#include "base/rand_util.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/system/sys_info.h"
+#include "base/task/thread_pool.h"
+#include "base/threading/thread_restrictions.h"
+#include "build/build_config.h"
+#include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_switches.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics_service.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics_service_factory.h"
+#include "chrome/browser/browseros/server/browseros_server_config.h"
+#include "chrome/browser/browseros/server/browseros_server_prefs.h"
+#include "chrome/browser/browseros/server/browseros_server_updater.h"
+#include "chrome/browser/browseros/server/browseros_server_utils.h"
+#include "chrome/browser/browseros/server/health_checker.h"
+#include "chrome/browser/browseros/server/health_checker_impl.h"
+#include "chrome/browser/browseros/server/process_controller.h"
+#include "chrome/browser/browseros/server/process_controller_impl.h"
+#include "chrome/browser/browseros/server/server_state_store.h"
+#include "chrome/browser/browseros/server/server_state_store_impl.h"
+#include "chrome/browser/browseros/server/server_updater.h"
+#include "chrome/browser/profiles/profile.h"
+#include "chrome/browser/profiles/profile_manager.h"
+#include "chrome/common/chrome_paths.h"
+#include "components/prefs/pref_change_registrar.h"
+#include "components/prefs/pref_service.h"
+#include "components/version_info/version_info.h"
+#include "content/public/browser/devtools_agent_host.h"
+#include "content/public/browser/devtools_socket_factory.h"
+#include "net/base/address_family.h"
+#include "net/base/ip_address.h"
+#include "net/base/ip_endpoint.h"
+#include "net/base/net_errors.h"
+#include "net/base/port_util.h"
+#include "net/log/net_log_source.h"
+#include "net/socket/tcp_server_socket.h"
+#include "net/socket/tcp_socket.h"
+
+namespace {
+
+constexpr int kBackLog = 10;
+
+constexpr base::TimeDelta kHealthCheckInterval = base::Seconds(30);
+constexpr base::TimeDelta kProcessCheckInterval = base::Seconds(10);
+
+// Crash tracking: if server crashes within grace period, count as startup failure
+constexpr base::TimeDelta kStartupGracePeriod = base::Seconds(30);
+constexpr int kMaxStartupFailures = 3;
+
+// Exit codes from BrowserOS server (must match packages/shared/src/constants/exit-codes.ts)
+// - 0 (SUCCESS): Clean shutdown, don't restart
+// - 1 (GENERAL_ERROR): Restart with same ports (default case, no const needed)
+// - 2 (PORT_CONFLICT): Restart with all ports revalidated
+constexpr int kExitCodeSuccess = 0;
+constexpr int kExitCodePortConflict = 2;
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
+BrowserOSServerManager::BrowserOSServerManager()
+    : process_controller_(std::make_unique<ProcessControllerImpl>()),
+      state_store_(std::make_unique<ServerStateStoreImpl>()),
+      health_checker_(std::make_unique<HealthCheckerImpl>()),
+      local_state_(g_browser_process ? g_browser_process->local_state()
+                                     : nullptr) {}
+
+BrowserOSServerManager::BrowserOSServerManager(
+    std::unique_ptr<ProcessController> process_controller,
+    std::unique_ptr<ServerStateStore> state_store,
+    std::unique_ptr<HealthChecker> health_checker,
+    std::unique_ptr<ServerUpdater> updater,
+    PrefService* local_state)
+    : process_controller_(std::move(process_controller)),
+      state_store_(std::move(state_store)),
+      health_checker_(std::move(health_checker)),
+      local_state_(local_state),
+      updater_(std::move(updater)) {}
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
+bool BrowserOSServerManager::RecoverFromOrphan() {
+  // Allow blocking for state file and process operations
+  base::ScopedAllowBlocking allow_blocking;
+
+  // Read state file
+  std::optional<server_utils::ServerState> state = state_store_->Read();
+  if (!state) {
+    LOG(INFO) << "browseros: No orphan state file found";
+    return false;
+  }
+
+  LOG(INFO) << "browseros: Found state file - PID: " << state->pid
+            << ", creation_time: " << state->creation_time;
+
+  // Check if process exists
+  if (!server_utils::ProcessExists(state->pid)) {
+    LOG(INFO) << "browseros: Process " << state->pid << " no longer exists";
+    state_store_->Delete();
+    return false;
+  }
+
+  // Validate creation time to handle PID reuse
+  std::optional<int64_t> actual_creation_time =
+      server_utils::GetProcessCreationTime(state->pid);
+  if (!actual_creation_time) {
+    LOG(WARNING) << "browseros: Could not get creation time for PID "
+                 << state->pid;
+    state_store_->Delete();
+    return false;
+  }
+
+  if (*actual_creation_time != state->creation_time) {
+    LOG(INFO) << "browseros: PID " << state->pid << " was reused "
+              << "(expected creation_time: " << state->creation_time
+              << ", actual: " << *actual_creation_time << ")";
+    state_store_->Delete();
+    return false;
+  }
+
+  // This is our orphan - kill it
+  LOG(INFO) << "browseros: Killing orphan server (PID: " << state->pid << ")";
+  constexpr base::TimeDelta kGracefulTimeout = base::Seconds(2);
+  bool killed = server_utils::KillProcess(state->pid, kGracefulTimeout);
+
+  if (killed) {
+    LOG(INFO) << "browseros: Orphan server killed successfully";
+  } else {
+    LOG(WARNING) << "browseros: Failed to kill orphan server, proceeding anyway";
+  }
+
+  state_store_->Delete();
+  return killed;
+}
+
+void BrowserOSServerManager::LoadPortsFromPrefs() {
+  if (!local_state_) {
+    ports_.cdp = browseros_server::kDefaultCDPPort;
+    ports_.mcp = browseros_server::kDefaultMCPPort;
+    ports_.extension = browseros_server::kDefaultExtensionPort;
+    allow_remote_in_mcp_ = false;
+    return;
+  }
+
+  ports_.cdp = local_state_->GetInteger(browseros_server::kCDPServerPort);
+  if (ports_.cdp <= 0) {
+    ports_.cdp = browseros_server::kDefaultCDPPort;
+  }
+
+  ports_.mcp = local_state_->GetInteger(browseros_server::kMCPServerPort);
+  if (ports_.mcp <= 0) {
+    ports_.mcp = browseros_server::kDefaultMCPPort;
+  }
+
+  ports_.extension = local_state_->GetInteger(browseros_server::kExtensionServerPort);
+  if (ports_.extension <= 0) {
+    ports_.extension = browseros_server::kDefaultExtensionPort;
+  }
+
+  allow_remote_in_mcp_ = local_state_->GetBoolean(browseros_server::kAllowRemoteInMCP);
+
+  LOG(INFO) << "browseros: Loaded ports from prefs - " << ports_.DebugString();
+}
+
+void BrowserOSServerManager::SetupPrefObservers() {
+  if (!local_state_ || pref_change_registrar_) {
+    return;  // No prefs or already set up
+  }
+
+  pref_change_registrar_ = std::make_unique<PrefChangeRegistrar>();
+  pref_change_registrar_->Init(local_state_);
+  pref_change_registrar_->Add(
+      browseros_server::kAllowRemoteInMCP,
+      base::BindRepeating(&BrowserOSServerManager::OnAllowRemoteInMCPChanged,
+                          base::Unretained(this)));
+  pref_change_registrar_->Add(
+      browseros_server::kRestartServerRequested,
+      base::BindRepeating(
+          &BrowserOSServerManager::OnRestartServerRequestedChanged,
+          base::Unretained(this)));
+}
+
+void BrowserOSServerManager::ResolvePortsForStartup() {
+  // Track assigned ports to prevent collisions between our services
+  std::set<int> assigned_ports;
+
+  // CDP: Chrome binds this port, so find available
+  ports_.cdp = server_utils::FindAvailablePort(ports_.cdp, assigned_ports);
+  assigned_ports.insert(ports_.cdp);
+
+  // MCP: Use saved value directly - do NOT revalidate.
+  // If port is taken, server will exit with PORT_CONFLICT (code 2),
+  // which triggers full revalidation via RevalidatePortsForRestart().
+  assigned_ports.insert(ports_.mcp);
+
+  // Extension: Find available port
+  ports_.extension =
+      server_utils::FindAvailablePort(ports_.extension, assigned_ports);
+
+  LOG(INFO) << "browseros: Resolved ports for startup - " << ports_.DebugString()
+            << " (MCP stable)";
+}
+
+void BrowserOSServerManager::ApplyCommandLineOverrides() {
+  base::CommandLine* command_line = base::CommandLine::ForCurrentProcess();
+
+  int cdp_override = GetPortOverrideFromCommandLine(
+      command_line, browseros::kCDPPort, "CDP port");
+  if (cdp_override > 0) {
+    ports_.cdp = cdp_override;
+  }
+
+  int mcp_override = GetPortOverrideFromCommandLine(
+      command_line, browseros::kMCPPort, "MCP port");
+  if (mcp_override > 0) {
+    ports_.mcp = mcp_override;
+  }
+
+  int extension_override = GetPortOverrideFromCommandLine(
+      command_line, browseros::kExtensionPort, "Extension port");
+  if (extension_override > 0) {
+    ports_.extension = extension_override;
+  }
+
+  LOG(INFO) << "browseros: Final ports after CLI overrides - "
+            << ports_.DebugString();
+}
+
+void BrowserOSServerManager::SavePortsToPrefs() {
+  if (!local_state_) {
+    LOG(WARNING) << "browseros: SavePortsToPrefs - no prefs available, skipping save";
+    return;
+  }
+
+  local_state_->SetInteger(browseros_server::kCDPServerPort, ports_.cdp);
+  local_state_->SetInteger(browseros_server::kMCPServerPort, ports_.mcp);
+  local_state_->SetInteger(browseros_server::kExtensionServerPort, ports_.extension);
+
+  LOG(INFO) << "browseros: Saving to prefs - " << ports_.DebugString();
+}
+
+void BrowserOSServerManager::Start() {
+  if (is_running_) {
+    LOG(INFO) << "browseros: BrowserOS server already running";
+    return;
+  }
+
+  // Initialize ports in clean steps:
+  // 1. Load saved values from prefs
+  // 2. Set up pref change observers
+  // 3. Resolve ports for startup (MCP stays stable, others find available)
+  // 4. Apply CLI overrides
+  // 5. Save final values to prefs
+  LoadPortsFromPrefs();
+  SetupPrefObservers();
+  ResolvePortsForStartup();
+  ApplyCommandLineOverrides();
+  SavePortsToPrefs();
+
+  base::CommandLine* command_line = base::CommandLine::ForCurrentProcess();
+  if (command_line->HasSwitch(browseros::kDisableServer)) {
+    LOG(INFO) << "browseros: BrowserOS server disabled via command line";
+    return;
+  }
+
+  // Try to acquire system-wide lock
+  if (!AcquireLock()) {
+    return;  // Another Chrome process already owns the server
+  }
+
+  // Kill any orphan server from a previous crash (must be after lock, before launch)
+  // This frees the ports so we can reuse them from prefs.
+  RecoverFromOrphan();
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
+  is_running_ = false;
+
+  LOG(INFO) << "browseros: Stopping BrowserOS server";
+  health_check_timer_.Stop();
+  process_check_timer_.Stop();
+
+  // Stop the updater
+  if (updater_) {
+    updater_->Stop();
+    updater_.reset();
+  }
+
+  // Use wait=false for shutdown - just send kill signal, don't block UI thread
+  TerminateBrowserOSProcess(/*wait=*/false);
+
+  // Delete state file - clean shutdown means no orphan to recover
+  {
+    base::ScopedAllowBlocking allow_blocking;
+    state_store_->Delete();
+  }
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
+  LOG(INFO) << "browseros: Starting CDP server on port " << ports_.cdp;
+
+  content::DevToolsAgentHost::StartRemoteDebuggingServer(
+      std::make_unique<CDPServerSocketFactory>(ports_.cdp),
+      base::FilePath(),
+      base::FilePath());
+
+  LOG(INFO) << "browseros: CDP WebSocket server started at ws://127.0.0.1:"
+            << ports_.cdp;
+  LOG(INFO) << "browseros: " << ports_.DebugString()
+            << " (allow_remote: " << (allow_remote_in_mcp_ ? "true" : "false")
+            << ")";
+}
+
+void BrowserOSServerManager::StopCDPServer() {
+  if (ports_.cdp == 0) {
+    return;
+  }
+
+  LOG(INFO) << "browseros: Stopping CDP server";
+  content::DevToolsAgentHost::StopRemoteDebuggingServer();
+  ports_.cdp = 0;
+}
+
+ServerLaunchConfig BrowserOSServerManager::BuildLaunchConfig() {
+  ServerLaunchConfig config;
+
+  // Paths: use updater's best paths if available, otherwise bundled
+  config.paths.fallback_exe = GetBrowserOSServerExecutablePath();
+  config.paths.fallback_resources = GetBrowserOSServerResourcesPath();
+  config.paths.execution = GetBrowserOSExecutionDir();
+
+  if (updater_) {
+    config.paths.exe = updater_->GetBestServerBinaryPath();
+    config.paths.resources = updater_->GetBestServerResourcesPath();
+  } else {
+    config.paths.exe = config.paths.fallback_exe;
+    config.paths.resources = config.paths.fallback_resources;
+  }
+
+  // Ports: copy from our member
+  config.ports = ports_;
+
+  // Identity: gather version info
+  config.identity.browseros_version =
+      std::string(version_info::GetBrowserOSVersionNumber());
+  config.identity.chromium_version =
+      std::string(version_info::GetVersionNumber());
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
+        config.identity.install_id = metrics_service->GetInstallId();
+      }
+    }
+  }
+
+  // Flags
+  config.allow_remote_in_mcp = allow_remote_in_mcp_;
+
+  return config;
+}
+
+void BrowserOSServerManager::LaunchBrowserOSProcess() {
+  ServerLaunchConfig config = BuildLaunchConfig();
+
+  if (config.paths.execution.empty()) {
+    LOG(ERROR) << "browseros: Failed to resolve execution directory";
+    return;
+  }
+
+  LOG(INFO) << "browseros: Launching server - " << config.DebugString();
+
+  // Capture process_controller for use in lambda (raw pointer is safe since
+  // the lambda runs as part of PostTaskAndReplyWithResult which will invoke
+  // OnProcessLaunched back on this thread, and we own process_controller_)
+  ProcessController* pc = process_controller_.get();
+
+  // Post blocking work to background thread, get result back on UI thread
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_BLOCKING},
+      base::BindOnce(&ProcessController::Launch, base::Unretained(pc), config),
+      base::BindOnce(&BrowserOSServerManager::OnProcessLaunched,
+                     weak_factory_.GetWeakPtr()));
+}
+
+void BrowserOSServerManager::OnProcessLaunched(LaunchResult result) {
+  bool was_updating = is_updating_;
+
+  // If we fell back to bundled binary, invalidate downloaded version
+  if (result.used_fallback && updater_) {
+    updater_->InvalidateDownloadedVersion();
+  }
+
+  if (!result.process.IsValid()) {
+    LOG(ERROR) << "browseros: Failed to launch BrowserOS server";
+    // Don't stop CDP server - it's independent and may be used by other things
+    // Leave system in degraded state (CDP up, no browseros_server) rather than
+    // completely broken state (no CDP, no server)
+    is_restarting_ = false;
+
+    // Notify updater of failure if this was an update restart
+    if (was_updating) {
+      is_updating_ = false;
+      if (update_complete_callback_) {
+        std::move(update_complete_callback_).Run(false);
+      }
+    }
+    return;
+  }
+
+  process_ = std::move(result.process);
+  is_running_ = true;
+  last_launch_time_ = base::TimeTicks::Now();
+
+  LOG(INFO) << "browseros: BrowserOS server started with PID: " << process_.Pid();
+  LOG(INFO) << "browseros: " << ports_.DebugString();
+
+  // Write state file for orphan recovery on next startup
+  {
+    base::ScopedAllowBlocking allow_blocking;
+    std::optional<int64_t> creation_time =
+        server_utils::GetProcessCreationTime(process_.Pid());
+    if (creation_time) {
+      server_utils::ServerState state;
+      state.pid = process_.Pid();
+      state.creation_time = *creation_time;
+      if (!state_store_->Write(state)) {
+        LOG(WARNING) << "browseros: Failed to write server state file";
+      }
+    } else {
+      LOG(WARNING)
+          << "browseros: Could not get process creation time for state file";
+    }
+  }
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
+    if (local_state_ &&
+        local_state_->GetBoolean(browseros_server::kRestartServerRequested)) {
+      local_state_->SetBoolean(browseros_server::kRestartServerRequested, false);
+      LOG(INFO) << "browseros: Restart completed, reset restart_requested pref";
+    }
+  }
+
+  // Notify updater of success if this was an update restart
+  if (was_updating) {
+    is_updating_ = false;
+    if (update_complete_callback_) {
+      std::move(update_complete_callback_).Run(true);
+    }
+  }
+
+  // Start the updater (if not already running and not disabled)
+  if (!updater_) {
+    if (base::CommandLine::ForCurrentProcess()->HasSwitch(
+            browseros::kDisableServerUpdater)) {
+      LOG(INFO) << "browseros: Server updater disabled via command line";
+    } else {
+      updater_ =
+          std::make_unique<browseros_server::BrowserOSServerUpdater>(this);
+      updater_->Start();
+    }
+  }
+}
+
+void BrowserOSServerManager::TerminateBrowserOSProcess(bool wait) {
+  if (!process_.IsValid()) {
+    return;
+  }
+
+  process_controller_->Terminate(&process_, wait);
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
+  // Handle clean shutdown (exit code 0) - don't restart
+  if (exit_code == kExitCodeSuccess) {
+    LOG(INFO) << "browseros: Server exited cleanly (code 0), not restarting";
+    return;
+  }
+
+  // Crash tracking: check if this was a startup failure (only for non-clean exits)
+  base::TimeDelta uptime = base::TimeTicks::Now() - last_launch_time_;
+  if (uptime < kStartupGracePeriod) {
+    consecutive_startup_failures_++;
+    LOG(WARNING) << "browseros: Startup failure detected (uptime: "
+                 << uptime.InSeconds() << "s, consecutive failures: "
+                 << consecutive_startup_failures_ << ")";
+
+    if (consecutive_startup_failures_ >= kMaxStartupFailures) {
+      LOG(ERROR) << "browseros: Too many startup failures ("
+                 << consecutive_startup_failures_
+                 << "), invalidating downloaded version";
+      if (updater_) {
+        updater_->InvalidateDownloadedVersion();
+      }
+      consecutive_startup_failures_ = 0;
+    }
+  } else {
+    // Process ran past grace period, reset failure counter
+    consecutive_startup_failures_ = 0;
+  }
+
+  // Prevent concurrent restarts (e.g., if RestartBrowserOSProcess is in progress)
+  if (is_restarting_) {
+    LOG(INFO) << "browseros: Restart already in progress, skipping";
+    return;
+  }
+  is_restarting_ = true;
+
+  // Determine restart strategy based on exit code
+  bool revalidate_all = (exit_code == kExitCodePortConflict);
+
+  if (revalidate_all) {
+    LOG(WARNING) << "browseros: Port conflict (code 2), will revalidate all port";
+  } else {
+    LOG(WARNING) << "browseros: Server exited (code " << exit_code
+                 << "), restarting with same ports";
+  }
+
+  // Capture current ports for background thread
+  ServerPorts current_ports = ports_;
+
+  // Revalidate ports on background thread, then launch on UI thread
+  // Process is already dead, no need to terminate
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_BLOCKING},
+      base::BindOnce(&BrowserOSServerManager::RevalidatePortsForRestart,
+                     base::Unretained(this), current_ports, revalidate_all),
+      base::BindOnce(&BrowserOSServerManager::OnPortsRevalidated,
+                     weak_factory_.GetWeakPtr()));
+}
+
+void BrowserOSServerManager::CheckServerHealth() {
+  if (!is_running_) {
+    return;
+  }
+
+  health_checker_->CheckHealth(
+      ports_.mcp,
+      base::BindOnce(&BrowserOSServerManager::OnHealthCheckComplete,
+                     weak_factory_.GetWeakPtr()));
+}
+
+void BrowserOSServerManager::CheckProcessStatus() {
+  if (!is_running_ || !process_.IsValid() || is_restarting_) {
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
+void BrowserOSServerManager::OnHealthCheckComplete(bool success) {
+  if (!is_running_) {
+    return;
+  }
+
+  if (success) {
+    LOG(INFO) << "browseros: Health check passed";
+    consecutive_health_check_failures_ = 0;
+    return;
+  }
+
+  consecutive_health_check_failures_++;
+  LOG(WARNING) << "browseros: Health check failed ("
+               << consecutive_health_check_failures_ << " consecutive)";
+
+  bool revalidate_all = (consecutive_health_check_failures_ >= 3);
+  if (revalidate_all) {
+    LOG(WARNING)
+        << "browseros: 3 consecutive failures, will revalidate all ports";
+    consecutive_health_check_failures_ = 0;
+  }
+  last_restart_revalidated_all_ports_ = revalidate_all;
+
+  RestartBrowserOSProcess(revalidate_all);
+}
+
+void BrowserOSServerManager::RestartBrowserOSProcess(bool revalidate_all_ports) {
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
+  ServerPorts current_ports = ports_;
+
+  // Kill process on background thread, wait for port release, revalidate, launch
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_BLOCKING},
+      base::BindOnce(
+          [](BrowserOSServerManager* manager, ServerPorts current,
+             bool revalidate_all) -> ServerPorts {
+            manager->TerminateBrowserOSProcess(/*wait=*/true);
+            return manager->RevalidatePortsForRestart(current, revalidate_all);
+          },
+          base::Unretained(this), current_ports, revalidate_all_ports),
+      base::BindOnce(&BrowserOSServerManager::OnPortsRevalidated,
+                     weak_factory_.GetWeakPtr()));
+}
+
+ServerPorts BrowserOSServerManager::RevalidatePortsForRestart(
+    const ServerPorts& current,
+    bool revalidate_all) {
+  // CDP port is excluded - it's still bound by Chrome's DevTools server
+  std::set<int> excluded_ports;
+  excluded_ports.insert(current.cdp);
+
+  ServerPorts result;
+  result.cdp = current.cdp;  // CDP never changes during restart
+
+  if (revalidate_all) {
+    // PORT_CONFLICT: server tried binding for 30s, port is truly blocked.
+    // Revalidate ALL ports - FindAvailablePort will increment if needed.
+    result.mcp = server_utils::FindAvailablePort(current.mcp, excluded_ports);
+    excluded_ports.insert(result.mcp);
+
+    result.extension =
+        server_utils::FindAvailablePort(current.extension, excluded_ports);
+
+    LOG(INFO) << "browseros: Ports revalidated (conflict) - "
+              << "MCP: " << current.mcp << " -> " << result.mcp
+              << ", Extension: " << current.extension << " -> "
+              << result.extension;
+  } else {
+    // Normal restart: trust MCP port will be available after TIME_WAIT.
+    // Exclude it so other ports don't accidentally take it.
+    result.mcp = current.mcp;
+    excluded_ports.insert(result.mcp);
+
+    result.extension =
+        server_utils::FindAvailablePort(current.extension, excluded_ports);
+  }
+
+  return result;
+}
+
+void BrowserOSServerManager::OnPortsRevalidated(ServerPorts new_ports) {
+  bool ports_changed = (new_ports != ports_);
+
+  if (ports_changed) {
+    LOG(INFO) << "browseros: Ports changed during revalidation - "
+              << "old: " << ports_.DebugString()
+              << ", new: " << new_ports.DebugString();
+    ports_ = new_ports;
+    SavePortsToPrefs();
+  }
+
+  // Note: is_restarting_ is cleared in OnProcessLaunched() after launch completes
+  LaunchBrowserOSProcess();
+}
+
+void BrowserOSServerManager::RestartServerForUpdate(
+    UpdateCompleteCallback callback) {
+  LOG(INFO) << "browseros: Restarting server for OTA update";
+
+  // Prevent multiple concurrent restarts
+  if (is_restarting_ || is_updating_) {
+    LOG(WARNING) << "browseros: Restart already in progress, failing update";
+    std::move(callback).Run(false);
+    return;
+  }
+
+  is_updating_ = true;
+  update_complete_callback_ = std::move(callback);
+
+  // Use same restart flow as RestartBrowserOSProcess
+  is_restarting_ = true;
+  health_check_timer_.Stop();
+  process_check_timer_.Stop();
+
+  ServerPorts current_ports = ports_;
+
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_BLOCKING},
+      base::BindOnce(
+          [](BrowserOSServerManager* manager,
+             ServerPorts current) -> ServerPorts {
+            manager->TerminateBrowserOSProcess(/*wait=*/true);
+            return manager->RevalidatePortsForRestart(current,
+                                                      /*revalidate_all=*/false);
+          },
+          base::Unretained(this), current_ports),
+      base::BindOnce(&BrowserOSServerManager::OnPortsRevalidated,
+                     weak_factory_.GetWeakPtr()));
+}
+
+void BrowserOSServerManager::OnAllowRemoteInMCPChanged() {
+  if (!is_running_ || !local_state_) {
+    return;
+  }
+
+  bool new_value = local_state_->GetBoolean(browseros_server::kAllowRemoteInMCP);
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
+  if (!local_state_) {
+    return;
+  }
+
+  bool restart_requested =
+      local_state_->GetBoolean(browseros_server::kRestartServerRequested);
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
+base::FilePath BrowserOSServerManager::GetBrowserOSServerResourcesPath() const {
+  // Check for command-line override first
+  base::CommandLine* command_line = base::CommandLine::ForCurrentProcess();
+  if (command_line->HasSwitch(browseros::kServerResourcesDir)) {
+    base::FilePath custom_path =
+        command_line->GetSwitchValuePath(browseros::kServerResourcesDir);
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
