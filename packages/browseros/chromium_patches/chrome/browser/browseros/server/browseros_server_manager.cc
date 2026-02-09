diff --git a/chrome/browser/browseros/server/browseros_server_manager.cc b/chrome/browser/browseros/server/browseros_server_manager.cc
new file mode 100644
index 0000000000000..87a0c20a2c67e
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_manager.cc
@@ -0,0 +1,1062 @@
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
+#include "base/functional/callback_helpers.h"
+#include "base/files/file_path.h"
+#include "base/files/file_util.h"
+#include "base/logging.h"
+#include "base/path_service.h"
+#include "base/rand_util.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/system/sys_info.h"
+#include "base/task/thread_pool.h"
+#include "content/public/browser/browser_thread.h"
+#include "base/threading/thread_restrictions.h"
+#include "build/build_config.h"
+#include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_switches.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics_service.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics_service_factory.h"
+#include "chrome/browser/browseros/server/browseros_server_config.h"
+#include "chrome/browser/browseros/server/browseros_server_prefs.h"
+#include "chrome/browser/browseros/server/browseros_server_proxy.h"
+#include "chrome/browser/browseros/server/browseros_server_updater.h"
+#include "chrome/browser/browseros/server/browseros_server_utils.h"
+#include "chrome/browser/browseros/server/health_checker.h"
+#include "chrome/browser/browseros/server/health_checker_impl.h"
+#include "chrome/browser/browseros/server/process_controller.h"
+#include "chrome/browser/browseros/server/process_controller_impl.h"
+#include "chrome/browser/browseros/server/server_state_store.h"
+#include "chrome/browser/browseros/server/server_state_store_impl.h"
+#include "chrome/browser/browseros/server/server_updater.h"
+#include "chrome/browser/net/system_network_context_manager.h"
+#include "services/network/public/cpp/shared_url_loader_factory.h"
+#include "chrome/browser/profiles/profile.h"
+#include "chrome/browser/profiles/profile_manager.h"
+#include "chrome/common/chrome_paths.h"
+#include "components/prefs/pref_change_registrar.h"
+#include "components/prefs/pref_service.h"
+#include "components/version_info/version_info.h"
+#include "content/public/browser/browser_thread.h"
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
+constexpr base::TimeDelta kProcessCheckInterval = base::Seconds(5);
+
+constexpr base::TimeDelta kStartupGracePeriod = base::Seconds(30);
+constexpr int kMaxStartupFailures = 3;
+
+constexpr int kExitCodeSuccess = 0;
+
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
+  std::unique_ptr<net::ServerSocket> CreateForHttpServer() override {
+    return CreateLocalHostServerSocket(port_);
+  }
+
+  std::unique_ptr<net::ServerSocket> CreateForTethering(
+      std::string* name) override {
+    return nullptr;
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
+  base::ScopedAllowBlocking allow_blocking;
+
+  std::optional<server_utils::ServerState> state = state_store_->Read();
+  if (!state) {
+    LOG(INFO) << "browseros: No orphan state file found";
+    return false;
+  }
+
+  LOG(INFO) << "browseros: Found state file - PID: " << state->pid
+            << ", creation_time: " << state->creation_time;
+
+  if (!server_utils::ProcessExists(state->pid)) {
+    LOG(INFO) << "browseros: Process " << state->pid << " no longer exists";
+    state_store_->Delete();
+    return false;
+  }
+
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
+    ports_.proxy = browseros_server::kDefaultProxyPort;
+    ports_.server = browseros_server::kDefaultServerPort;
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
+  // Migration: read old kMCPServerPort into proxy if kProxyPort not yet set
+  int proxy_port = local_state_->GetInteger(browseros_server::kProxyPort);
+  if (proxy_port <= 0) {
+    int old_mcp = local_state_->GetInteger(browseros_server::kMCPServerPort);
+    if (old_mcp > 0) {
+      proxy_port = old_mcp;
+      LOG(INFO) << "browseros: Migrated old MCP port " << old_mcp
+                << " to proxy port";
+    } else {
+      proxy_port = browseros_server::kDefaultProxyPort;
+    }
+  }
+  ports_.proxy = proxy_port;
+
+  ports_.server = local_state_->GetInteger(browseros_server::kServerPort);
+  if (ports_.server <= 0) {
+    ports_.server = browseros_server::kDefaultServerPort;
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
+    return;
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
+  base::CommandLine* command_line = base::CommandLine::ForCurrentProcess();
+  std::set<int> assigned_ports;
+
+  // Skip FindAvailablePort for CLI-overridden ports — trust the developer.
+  bool cdp_fixed = command_line->HasSwitch(browseros::kCDPPort);
+  bool proxy_fixed = command_line->HasSwitch(browseros::kProxyPort);
+  bool server_fixed = command_line->HasSwitch(browseros::kServerPort);
+  bool extension_fixed = command_line->HasSwitch(browseros::kExtensionPort);
+
+  if (cdp_fixed) {
+    assigned_ports.insert(ports_.cdp);
+  } else {
+    ports_.cdp = server_utils::FindAvailablePort(ports_.cdp, assigned_ports);
+    assigned_ports.insert(ports_.cdp);
+  }
+
+  if (proxy_fixed) {
+    assigned_ports.insert(ports_.proxy);
+  } else {
+    ports_.proxy = server_utils::FindAvailablePort(ports_.proxy, assigned_ports,
+                                                   /*allow_reuse=*/true);
+    assigned_ports.insert(ports_.proxy);
+  }
+
+  if (server_fixed) {
+    assigned_ports.insert(ports_.server);
+  } else {
+    ports_.server = server_utils::FindAvailablePort(
+        browseros_server::kDefaultServerPort, assigned_ports);
+    assigned_ports.insert(ports_.server);
+  }
+
+  if (extension_fixed) {
+    assigned_ports.insert(ports_.extension);
+  } else {
+    ports_.extension = server_utils::FindAvailablePort(
+        browseros_server::kDefaultExtensionPort, assigned_ports);
+  }
+
+  LOG(INFO) << "browseros: Resolved ports for startup - " << ports_.DebugString();
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
+  int proxy_override = GetPortOverrideFromCommandLine(
+      command_line, browseros::kProxyPort, "proxy port");
+  if (proxy_override > 0) {
+    ports_.proxy = proxy_override;
+  }
+
+  int server_override = GetPortOverrideFromCommandLine(
+      command_line, browseros::kServerPort, "server port");
+  if (server_override > 0) {
+    ports_.server = server_override;
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
+  local_state_->SetInteger(browseros_server::kProxyPort, ports_.proxy);
+  local_state_->SetInteger(browseros_server::kServerPort, ports_.server);
+  local_state_->SetInteger(browseros_server::kExtensionServerPort, ports_.extension);
+
+  // DEPRECATED: keep mcp_port in sync with server port for backward compat
+  local_state_->SetInteger(browseros_server::kMCPServerPort, ports_.server);
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
+  // Phase 1: Load user intent (prefs + CLI overrides).
+  // Save stable port preferences so CLI overrides are persisted even when
+  // the server is disabled or we lose the lock.
+  LoadPortsFromPrefs();
+  SetupPrefObservers();
+  ApplyCommandLineOverrides();
+  SavePortsToPrefs();
+
+  base::CommandLine* command_line = base::CommandLine::ForCurrentProcess();
+  if (command_line->HasSwitch(browseros::kDisableServer)) {
+    LOG(INFO) << "browseros: BrowserOS server disabled via command line";
+    return;
+  }
+
+  if (!AcquireLock()) {
+    return;
+  }
+
+  // Phase 2: We hold the lock — we're the active instance.
+  // Now resolve actual available ports and save the final values.
+  RecoverFromOrphan();
+  ResolvePortsForStartup();
+  SavePortsToPrefs();
+
+  LOG(INFO) << "browseros: Starting BrowserOS server";
+
+  StartCDPServer();
+  StartProxy();
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
+  if (updater_) {
+    updater_->Stop();
+    updater_.reset();
+  }
+
+  StopProxy();
+
+  TerminateBrowserOSProcess(base::DoNothing());
+
+  {
+    base::ScopedAllowBlocking allow_blocking;
+    state_store_->Delete();
+  }
+
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
+void BrowserOSServerManager::StartProxy() {
+  server_proxy_ = std::make_unique<BrowserOSServerProxy>();
+
+  // Clone the factory on the UI thread so it can be bound on the IO thread.
+  auto pending_factory =
+      g_browser_process->system_network_context_manager()
+          ->GetSharedURLLoaderFactory()
+          ->Clone();
+
+  content::GetIOThreadTaskRunner({})->PostTask(
+      FROM_HERE,
+      base::BindOnce(
+          [](BrowserOSServerProxy* proxy, int port,
+             std::unique_ptr<network::PendingSharedURLLoaderFactory> pending,
+             bool allow_remote) {
+            if (!proxy->Start(port, std::move(pending))) {
+              LOG(ERROR) << "browseros: Failed to start MCP proxy on port "
+                         << port;
+              return;
+            }
+            proxy->SetAllowRemote(allow_remote);
+          },
+          server_proxy_.get(), ports_.proxy, std::move(pending_factory),
+          allow_remote_in_mcp_));
+}
+
+void BrowserOSServerManager::StopProxy() {
+  if (server_proxy_) {
+    content::GetIOThreadTaskRunner({})->PostTask(
+        FROM_HERE,
+        base::BindOnce(
+            [](std::unique_ptr<BrowserOSServerProxy> proxy) {
+              proxy->Stop();
+              // proxy destroyed on IO thread
+            },
+            std::move(server_proxy_)));
+  }
+}
+
+ServerLaunchConfig BrowserOSServerManager::BuildLaunchConfig() {
+  ServerLaunchConfig config;
+
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
+  config.ports = ports_;
+
+  config.identity.browseros_version =
+      std::string(version_info::GetBrowserOSVersionNumber());
+  config.identity.chromium_version =
+      std::string(version_info::GetVersionNumber());
+
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
+  ProcessController* pc = process_controller_.get();
+
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
+  if (result.used_fallback && updater_) {
+    updater_->InvalidateDownloadedVersion();
+  }
+
+  if (!result.process.IsValid()) {
+    LOG(ERROR) << "browseros: Failed to launch BrowserOS server";
+    is_restarting_ = false;
+
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
+  // Point proxy at the new backend port (proxy lives on IO thread)
+  if (server_proxy_) {
+    content::GetIOThreadTaskRunner({})->PostTask(
+        FROM_HERE,
+        base::BindOnce(&BrowserOSServerProxy::SetBackendPort,
+                       base::Unretained(server_proxy_.get()), ports_.server));
+  }
+
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
+  health_check_timer_.Start(FROM_HERE, kHealthCheckInterval, this,
+                            &BrowserOSServerManager::CheckServerHealth);
+  process_check_timer_.Start(FROM_HERE, kProcessCheckInterval, this,
+                             &BrowserOSServerManager::CheckProcessStatus);
+
+  if (is_restarting_) {
+    is_restarting_ = false;
+    if (local_state_ &&
+        local_state_->GetBoolean(browseros_server::kRestartServerRequested)) {
+      local_state_->SetBoolean(browseros_server::kRestartServerRequested, false);
+      LOG(INFO) << "browseros: Restart completed, reset restart_requested pref";
+    }
+  }
+
+  if (was_updating) {
+    is_updating_ = false;
+    if (update_complete_callback_) {
+      std::move(update_complete_callback_).Run(true);
+    }
+  }
+
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
+void BrowserOSServerManager::TerminateBrowserOSProcess(
+    base::OnceCallback<void()> callback) {
+  if (!process_.IsValid()) {
+    std::move(callback).Run();
+    return;
+  }
+
+  LOG(INFO) << "browseros: Requesting graceful shutdown via HTTP";
+  health_checker_->RequestShutdown(
+      ports_.server,
+      base::BindOnce(&BrowserOSServerManager::OnTerminateHttpComplete,
+                     weak_factory_.GetWeakPtr(), std::move(callback)));
+}
+
+void BrowserOSServerManager::OnTerminateHttpComplete(
+    base::OnceCallback<void()> callback,
+    bool http_success) {
+  if (http_success) {
+    LOG(INFO) << "browseros: Graceful shutdown acknowledged, trusting exit";
+  } else {
+    LOG(WARNING) << "browseros: HTTP shutdown failed, sending SIGKILL";
+    if (process_.IsValid()) {
+      process_controller_->Terminate(&process_, /*wait=*/false);
+    }
+  }
+  std::move(callback).Run();
+}
+
+void BrowserOSServerManager::OnProcessExited(int exit_code) {
+  LOG(INFO) << "browseros: BrowserOS server exited with code: " << exit_code;
+  is_running_ = false;
+
+  health_check_timer_.Stop();
+  process_check_timer_.Stop();
+
+  if (exit_code == kExitCodeSuccess) {
+    LOG(INFO) << "browseros: Server exited cleanly (code 0), not restarting";
+    return;
+  }
+
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
+    consecutive_startup_failures_ = 0;
+  }
+
+  if (is_restarting_) {
+    LOG(INFO) << "browseros: Restart already in progress, skipping";
+    return;
+  }
+
+  LOG(WARNING) << "browseros: Server exited (code " << exit_code
+               << "), restarting with new ephemeral ports";
+  RestartBrowserOSProcess();
+}
+
+void BrowserOSServerManager::CheckServerHealth() {
+  if (!is_running_) {
+    return;
+  }
+
+  health_checker_->CheckHealth(
+      ports_.server,
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
+  VLOG(1) << "browseros: CheckProcessStatus PID: " << process_.Pid()
+          << ", WaitForExitWithTimeout returned: " << exited
+          << ", exit_code: " << exit_code;
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
+    return;
+  }
+
+  LOG(WARNING) << "browseros: Health check failed, restarting";
+  RestartBrowserOSProcess();
+}
+
+void BrowserOSServerManager::RestartBrowserOSProcess() {
+  LOG(INFO) << "browseros: Restarting BrowserOS server process";
+
+  if (is_restarting_) {
+    LOG(INFO) << "browseros: Restart already in progress, ignoring";
+    return;
+  }
+  is_restarting_ = true;
+
+  health_check_timer_.Stop();
+  process_check_timer_.Stop();
+
+  TerminateBrowserOSProcess(
+      base::BindOnce(&BrowserOSServerManager::ContinueRestartAfterTerminate,
+                     weak_factory_.GetWeakPtr()));
+}
+
+void BrowserOSServerManager::ContinueRestartAfterTerminate() {
+  base::ThreadPool::PostTaskAndReply(
+      FROM_HERE,
+      {base::MayBlock(), base::WithBaseSyncPrimitives(),
+       base::TaskPriority::USER_BLOCKING},
+      base::BindOnce(
+          [](BrowserOSServerManager* manager) {
+            constexpr base::TimeDelta kExitTimeout = base::Seconds(5);
+            int exit_code = 0;
+            bool exited = manager->process_controller_->WaitForExitWithTimeout(
+                &manager->process_, kExitTimeout, &exit_code);
+
+            if (!exited) {
+              LOG(WARNING) << "browseros: Process didn't exit in time, "
+                           << "sending SIGKILL";
+              manager->process_controller_->Terminate(&manager->process_,
+                                                      /*wait=*/true);
+            }
+          },
+          base::Unretained(this)),
+      base::BindOnce(
+          [](base::WeakPtr<BrowserOSServerManager> weak_manager) {
+            if (!weak_manager) {
+              return;
+            }
+            auto* manager = weak_manager.get();
+
+            // Pick new ephemeral ports for server and extension
+            // (unless CLI-overridden)
+            base::CommandLine* cl =
+                base::CommandLine::ForCurrentProcess();
+            std::set<int> assigned;
+            assigned.insert(manager->ports_.cdp);
+            assigned.insert(manager->ports_.proxy);
+
+            if (!cl->HasSwitch(browseros::kServerPort)) {
+              manager->ports_.server =
+                  server_utils::FindAvailablePort(
+                      browseros_server::kDefaultServerPort, assigned);
+            }
+            assigned.insert(manager->ports_.server);
+
+            if (!cl->HasSwitch(browseros::kExtensionPort)) {
+              manager->ports_.extension =
+                  server_utils::FindAvailablePort(
+                      browseros_server::kDefaultExtensionPort, assigned);
+            }
+
+            LOG(INFO) << "browseros: New ephemeral ports - "
+                      << manager->ports_.DebugString();
+
+            manager->SavePortsToPrefs();
+            manager->LaunchBrowserOSProcess();
+          },
+          weak_factory_.GetWeakPtr()));
+}
+
+void BrowserOSServerManager::RestartServerForUpdate(
+    UpdateCompleteCallback callback) {
+  LOG(INFO) << "browseros: Restarting server for OTA update";
+
+  if (is_restarting_ || is_updating_) {
+    LOG(WARNING) << "browseros: Restart already in progress, failing update";
+    std::move(callback).Run(false);
+    return;
+  }
+
+  is_updating_ = true;
+  update_complete_callback_ = std::move(callback);
+
+  is_restarting_ = true;
+  health_check_timer_.Stop();
+  process_check_timer_.Stop();
+
+  TerminateBrowserOSProcess(
+      base::BindOnce(&BrowserOSServerManager::ContinueUpdateAfterTerminate,
+                     weak_factory_.GetWeakPtr()));
+}
+
+void BrowserOSServerManager::ContinueUpdateAfterTerminate() {
+  base::ThreadPool::PostTaskAndReply(
+      FROM_HERE,
+      {base::MayBlock(), base::WithBaseSyncPrimitives(),
+       base::TaskPriority::USER_BLOCKING},
+      base::BindOnce(
+          [](BrowserOSServerManager* manager) {
+            constexpr base::TimeDelta kExitTimeout = base::Seconds(5);
+            int exit_code = 0;
+            bool exited = manager->process_controller_->WaitForExitWithTimeout(
+                &manager->process_, kExitTimeout, &exit_code);
+
+            if (!exited) {
+              LOG(WARNING) << "browseros: Process didn't exit for update, "
+                           << "sending SIGKILL";
+              manager->process_controller_->Terminate(&manager->process_,
+                                                      /*wait=*/true);
+            }
+          },
+          base::Unretained(this)),
+      base::BindOnce(
+          [](base::WeakPtr<BrowserOSServerManager> weak_manager) {
+            if (!weak_manager) {
+              return;
+            }
+            auto* manager = weak_manager.get();
+
+            base::CommandLine* cl =
+                base::CommandLine::ForCurrentProcess();
+            std::set<int> assigned;
+            assigned.insert(manager->ports_.cdp);
+            assigned.insert(manager->ports_.proxy);
+
+            if (!cl->HasSwitch(browseros::kServerPort)) {
+              manager->ports_.server =
+                  server_utils::FindAvailablePort(
+                      browseros_server::kDefaultServerPort, assigned);
+            }
+            assigned.insert(manager->ports_.server);
+
+            if (!cl->HasSwitch(browseros::kExtensionPort)) {
+              manager->ports_.extension =
+                  server_utils::FindAvailablePort(
+                      browseros_server::kDefaultExtensionPort, assigned);
+            }
+
+            manager->SavePortsToPrefs();
+            manager->LaunchBrowserOSProcess();
+          },
+          weak_factory_.GetWeakPtr()));
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
+    if (server_proxy_) {
+      content::GetIOThreadTaskRunner({})->PostTask(
+          FROM_HERE,
+          base::BindOnce(&BrowserOSServerProxy::SetAllowRemote,
+                         base::Unretained(server_proxy_.get()), new_value));
+    }
+
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
+  if (!restart_requested) {
+    return;
+  }
+
+  LOG(INFO) << "browseros: Server restart requested via preference";
+  RestartBrowserOSProcess();
+}
+
+base::FilePath BrowserOSServerManager::GetBrowserOSServerResourcesPath() const {
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
+  if (!base::PathService::Get(base::DIR_EXE, &exe_dir)) {
+    LOG(ERROR) << "browseros: Failed to get executable directory";
+    return base::FilePath();
+  }
+  exe_dir = exe_dir.DirName().Append("Resources");
+
+#elif BUILDFLAG(IS_WIN)
+  if (!base::PathService::Get(base::DIR_EXE, &exe_dir)) {
+    LOG(ERROR) << "browseros: Failed to get executable directory";
+    return base::FilePath();
+  }
+  exe_dir = exe_dir.AppendASCII(version_info::GetVersionNumber());
+
+#elif BUILDFLAG(IS_LINUX)
+  if (!base::PathService::Get(base::DIR_EXE, &exe_dir)) {
+    LOG(ERROR) << "browseros: Failed to get executable directory";
+    return base::FilePath();
+  }
+#endif
+
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
