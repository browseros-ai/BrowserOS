diff --git a/chrome/browser/browseros/server/browseros_server_manager.cc b/chrome/browser/browseros/server/browseros_server_manager.cc
new file mode 100644
index 0000000000000000000000000000000000000000..88c8769d97826bd909ada9b67cbba87d09782790
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_manager.cc
@@ -0,0 +1,1421 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_server_manager.h"
+
+#include <optional>
+#include <set>
+
+#include "base/check.h"
+#include "base/command_line.h"
+#include "base/files/file_path.h"
+#include "base/files/file_util.h"
+#include "base/functional/callback_helpers.h"
+#include "base/json/json_reader.h"
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
+#include "chrome/browser/browseros/metrics/browseros_metrics.h"
+#include "chrome/browser/browseros/server/browseros_server_config.h"
+#include "chrome/browser/browseros/server/browseros_server_constants.h"
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
+#include "chrome/common/chrome_paths.h"
+#include "components/prefs/pref_change_registrar.h"
+#include "components/prefs/pref_service.h"
+#include "components/version_info/version_info.h"
+#include "content/public/browser/browser_thread.h"
+#include "content/public/browser/devtools_agent_host.h"
+#include "content/public/browser/devtools_socket_factory.h"
+#include "content/public/common/content_switches.h"
+#include "net/base/address_family.h"
+#include "net/base/ip_address.h"
+#include "net/base/ip_endpoint.h"
+#include "net/base/net_errors.h"
+#include "net/base/port_util.h"
+#include "net/log/net_log_source.h"
+#include "net/socket/tcp_server_socket.h"
+#include "net/socket/tcp_socket.h"
+#include "net/traffic_annotation/network_traffic_annotation.h"
+#include "services/network/public/cpp/resource_request.h"
+#include "services/network/public/cpp/simple_url_loader.h"
+
+#if BUILDFLAG(IS_POSIX)
+#include <errno.h>
+#include <signal.h>
+#endif
+
+namespace {
+
+// A launch reply may be discarded after the browser's UI loop has stopped.
+// Keep child ownership in the reply itself until the manager adopts it, rather
+// than relying on a callback (or another UI task) to clean up a cancelled
+// launch.
+struct PendingServerLaunch {
+  browseros::LaunchResult result;
+
+  ~PendingServerLaunch() {
+    if (!result.process.IsValid()) {
+      return;
+    }
+#if BUILDFLAG(IS_POSIX)
+    kill(result.process.Pid(), SIGKILL);
+#else
+    result.process.Terminate(-1, false);
+#endif
+    int exit_code = 0;
+    result.process.WaitForExitWithTimeout(base::TimeDelta(), &exit_code);
+  }
+};
+
+constexpr int kBackLog = 10;
+
+constexpr base::TimeDelta kHealthCheckInterval = base::Seconds(30);
+constexpr base::TimeDelta kProcessCheckInterval = base::Seconds(5);
+constexpr base::TimeDelta kActivationRetryInterval = base::Seconds(30);
+constexpr base::TimeDelta kStartupHealthRetryInterval = base::Seconds(1);
+
+net::NetworkTrafficAnnotationTag GetStatusTrafficAnnotation() {
+  return net::DefineNetworkTrafficAnnotation("browseros_server_status", R"(
+    semantics {
+      sender: "BrowserOS Server Manager"
+      description:
+        "Checks whether the local server is idle before activating a prepared "
+        "update. Deferred updates are retried without another download."
+      trigger: "When a prepared update exists, then every 30 seconds until idle."
+      data: "No user data sent, just an HTTP GET to localhost."
+      destination: LOCAL
+    }
+    policy {
+      cookies_allowed: NO
+      setting: "Disabled when the managed server is disabled."
+      policy_exception_justification:
+        "Internal coordination to avoid interrupting active server requests."
+    })");
+}
+
+constexpr base::TimeDelta kStartupGracePeriod = base::Seconds(30);
+constexpr int kMaxStartupFailures = 3;
+constexpr int kMaxConsecutiveHealthCheckFailures = 2;
+
+constexpr int kExitCodeSuccess = 0;
+constexpr int kExitCodePortConflict = 2;
+
+int GetPortOverrideFromCommandLine(base::CommandLine* command_line,
+                                   const char* switch_name,
+                                   const char* port_name) {
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
+  lock_file_ =
+      base::File(lock_path, base::File::FLAG_OPEN_ALWAYS |
+                                base::File::FLAG_READ | base::File::FLAG_WRITE);
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
+    LOG(WARNING)
+        << "browseros: Failed to kill orphan server, proceeding anyway";
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
+    proxy_https_port_ = browseros_server::kDefaultProxyHttpsPort;
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
+  proxy_https_port_ =
+      local_state_->GetInteger(browseros_server::kProxyHttpsPort);
+  if (proxy_https_port_ <= 0) {
+    proxy_https_port_ = browseros_server::kDefaultProxyHttpsPort;
+  }
+
+  ports_.server = local_state_->GetInteger(browseros_server::kServerPort);
+  if (ports_.server <= 0) {
+    ports_.server = browseros_server::kDefaultServerPort;
+  }
+
+  allow_remote_in_mcp_ =
+      local_state_->GetBoolean(browseros_server::kAllowRemoteInMCP);
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
+  pref_change_registrar_->Add(
+      browseros_server::kProxyPort,
+      base::BindRepeating(&BrowserOSServerManager::OnProxyPortChanged,
+                          base::Unretained(this)));
+}
+
+void BrowserOSServerManager::ResolvePortsForStartup() {
+  base::CommandLine* command_line = base::CommandLine::ForCurrentProcess();
+  std::set<int> assigned_ports;
+
+  // Skip FindAvailablePort for CLI-overridden ports; trust the developer.
+  bool cdp_fixed = command_line->HasSwitch(browseros::kCDPPort);
+  bool proxy_fixed = command_line->HasSwitch(browseros::kProxyPort);
+  bool server_fixed = command_line->HasSwitch(browseros::kServerPort);
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
+  proxy_https_port_ = FindAvailableServerPort(proxy_https_port_, assigned_ports,
+                                              /*allow_reuse=*/true);
+  assigned_ports.insert(proxy_https_port_);
+
+  LOG(INFO) << "browseros: Resolved ports for startup - "
+            << ports_.DebugString();
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
+  LOG(INFO) << "browseros: Final ports after CLI overrides - "
+            << ports_.DebugString();
+}
+
+void BrowserOSServerManager::SavePortsToPrefs() {
+  if (!local_state_) {
+    LOG(WARNING)
+        << "browseros: SavePortsToPrefs - no prefs available, skipping save";
+    return;
+  }
+
+  local_state_->SetInteger(browseros_server::kCDPServerPort, ports_.cdp);
+  local_state_->SetInteger(browseros_server::kProxyPort, ports_.proxy);
+  local_state_->SetInteger(browseros_server::kProxyHttpsPort,
+                           proxy_https_port_);
+  local_state_->SetInteger(browseros_server::kServerPort, ports_.server);
+
+  // DEPRECATED: keep mcp_port in sync with server port for backward compat
+  local_state_->SetInteger(browseros_server::kMCPServerPort, ports_.server);
+
+  LOG(INFO) << "browseros: Saving to prefs - " << ports_.DebugString();
+}
+
+int BrowserOSServerManager::FindAvailableServerPort(
+    int starting_port,
+    const std::set<int>& excluded,
+    bool allow_reuse) {
+  if (port_finder_for_testing_) {
+    return port_finder_for_testing_.Run(starting_port, excluded, allow_reuse);
+  }
+
+  return server_utils::FindAvailablePort(starting_port, excluded, allow_reuse);
+}
+
+void BrowserOSServerManager::Start() {
+  if (started_ || stopping_ || is_running_) {
+    LOG(INFO) << "browseros: " << GetManagedServerDescriptor().log_name
+              << " already running";
+    return;
+  }
+
+  // Phase 1: Determine and resolve ports.
+  // Resolve runs unconditionally (even with --disable-server) so that
+  // CLI overrides are validated and persisted correctly.
+  LoadPortsFromPrefs();
+  SetupPrefObservers();
+  ApplyCommandLineOverrides();
+  ResolvePortsForStartup();
+  SavePortsToPrefs();
+
+  base::CommandLine* command_line = base::CommandLine::ForCurrentProcess();
+
+  // Phase 2: Bind CDP to the resolved port.
+  // Must happen AFTER resolution, otherwise FindAvailablePort detects our
+  // own CDP binding as "port in use" and reassigns to a different port,
+  // causing the sidecar to connect to a port where nothing listens.
+  if (!command_line->HasSwitch(::switches::kRemoteDebuggingPort)) {
+    StartCDPServer();
+  } else {
+    LOG(WARNING) << "browseros: Skipping managed CDP server - "
+                 << "--remote-debugging-port takes precedence";
+  }
+
+  if (command_line->HasSwitch(browseros::kDisableServer)) {
+    LOG(INFO) << "browseros: Managed server disabled via command line";
+    return;
+  }
+
+  // Phase 3: Sidecar lifecycle - lock, recover orphans, launch.
+  if (!AcquireLock()) {
+    return;
+  }
+
+  RecoverFromOrphan();
+
+  LOG(INFO) << "browseros: Starting " << GetManagedServerDescriptor().log_name;
+
+  started_ = true;
+  StartProxy();
+  ServerInstallation bundled;
+  bundled.executable = GetBrowserOSServerExecutablePath();
+  bundled.resources = GetBrowserOSServerResourcesPath();
+  version_store_ = std::make_unique<ServerVersionStore>(
+      GetBrowserOSExecutionDir(), std::move(bundled),
+      GetManagedServerDescriptor());
+  // The browser UI and proxy can start immediately. Only the sidecar launch
+  // waits for local discovery; network polling is not involved in this choice.
+  version_store_->Initialize(
+      base::BindOnce(&BrowserOSServerManager::OnVersionsInitialized,
+                     weak_factory_.GetWeakPtr()));
+}
+
+void BrowserOSServerManager::OnVersionsInitialized() {
+  if (!started_) {
+    return;
+  }
+  activation_retry_timer_.Start(
+      FROM_HERE, kActivationRetryInterval, this,
+      &BrowserOSServerManager::MaybeActivateReadyVersion);
+  LaunchBrowserOSProcess();
+}
+
+void BrowserOSServerManager::Stop() {
+  if (stopping_ || (!started_ && !is_running_ && !launch_in_progress_)) {
+    return;
+  }
+  started_ = false;
+  stopping_ = true;
+  weak_factory_.InvalidateWeakPtrs();
+  health_check_timer_.Stop();
+  process_check_timer_.Stop();
+  startup_health_retry_timer_.Stop();
+  startup_health_deadline_timer_.Stop();
+  activation_retry_timer_.Stop();
+  readiness_loader_.reset();
+  ClearRunningInstallation();
+
+  if (updater_) {
+    updater_->Stop();
+    updater_.reset();
+  }
+  updater_started_ = false;
+  version_io_pending_ = version_store_ != nullptr;
+  if (version_store_) {
+    version_store_->Shutdown(
+        base::BindOnce(&BrowserOSServerManager::OnVersionStoreStopped,
+                       process_weak_factory_.GetWeakPtr()));
+  }
+  next_installation_.reset();
+  StopProxy();
+  CompleteUpdate(false);
+
+  // An in-flight launch still owns a child-creation task. Its reply will stop
+  // that child before releasing the profile lock, rather than orphaning it.
+  if (!launch_in_progress_) {
+    TerminateBrowserOSProcess(
+        base::BindOnce(&BrowserOSServerManager::FinishStop,
+                       process_weak_factory_.GetWeakPtr()));
+  }
+}
+
+void BrowserOSServerManager::FinishStop() {
+  if (!stopping_ || launch_in_progress_ || termination_in_progress_ ||
+      version_io_pending_ || process_.IsValid()) {
+    return;
+  }
+  launched_installation_.reset();
+  is_restarting_ = false;
+  {
+    base::ScopedAllowBlocking allow_blocking;
+    state_store_->Delete();
+    if (lock_file_.IsValid()) {
+      lock_file_.Unlock();
+      lock_file_.Close();
+    }
+  }
+  stopping_ = false;
+  LOG(INFO) << "browseros: Managed server stopped; released profile lock";
+}
+
+void BrowserOSServerManager::OnVersionStoreStopped() {
+  version_store_.reset();
+  version_io_pending_ = false;
+  FinishStop();
+}
+
+void BrowserOSServerManager::ClearRunningInstallation() {
+  is_running_ = false;
+  running_installation_.reset();
+  if (local_state_) {
+    local_state_->SetString(browseros_server::kServerVersion, std::string());
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
+      std::make_unique<CDPServerSocketFactory>(ports_.cdp), base::FilePath(),
+      base::FilePath());
+
+  // Note: StartRemoteDebuggingServer binds on the IO thread asynchronously.
+  // A synchronous port check here would race with the IO-thread bind and
+  // produce false failures. The port was validated as available by
+  // ResolvePortsForStartup() immediately before this call.
+  LOG(INFO) << "browseros: CDP WebSocket server requested at ws://127.0.0.1:"
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
+  content::GetIOThreadTaskRunner({})->PostTask(
+      FROM_HERE, base::BindOnce(
+                     [](BrowserOSServerProxy* proxy, int http_port,
+                        int https_port, bool allow_remote) {
+                       if (!proxy->Start(http_port, https_port)) {
+                         LOG(ERROR)
+                             << "browseros: Failed to start MCP proxy on port "
+                             << http_port;
+                         return;
+                       }
+                       proxy->SetAllowRemote(allow_remote);
+                     },
+                     server_proxy_.get(), ports_.proxy, proxy_https_port_,
+                     allow_remote_in_mcp_));
+}
+
+void BrowserOSServerManager::StopProxy() {
+  if (server_proxy_) {
+    content::GetIOThreadTaskRunner({})->PostTask(
+        FROM_HERE, base::BindOnce(
+                       [](std::unique_ptr<BrowserOSServerProxy> proxy) {
+                         proxy->Stop();
+                         // proxy destroyed on IO thread
+                       },
+                       std::move(server_proxy_)));
+  }
+}
+
+ServerLaunchConfig BrowserOSServerManager::BuildLaunchConfig() {
+  ServerLaunchConfig config;
+  const ManagedServerDescriptor& descriptor = GetManagedServerDescriptor();
+
+  config.log_name = std::string(descriptor.log_name);
+  config.config_file_name =
+      base::FilePath::StringType(descriptor.config_file_name);
+  config.health_path = std::string(descriptor.health_path);
+  config.enable_updater = descriptor.enable_updater;
+
+  config.paths.fallback_exe = GetBrowserOSServerExecutablePath();
+  config.paths.fallback_resources = GetBrowserOSServerResourcesPath();
+  config.paths.execution = GetBrowserOSExecutionDir();
+
+  // Capture one installation for this process. Never resolve executable and
+  // resources independently or relabel this process when ready state changes.
+  if (version_store_ && version_store_->initialized()) {
+    launched_installation_ = next_installation_.has_value()
+                                 ? *next_installation_
+                                 : version_store_->GetBestAvailable();
+    next_installation_.reset();
+    config.paths.exe = launched_installation_->executable;
+    config.paths.resources = launched_installation_->resources;
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
+  config.allow_remote_in_mcp = allow_remote_in_mcp_;
+
+  return config;
+}
+
+void BrowserOSServerManager::LaunchBrowserOSProcess() {
+  if (!started_ || launch_in_progress_ || termination_in_progress_) {
+    return;
+  }
+  ServerLaunchConfig config = BuildLaunchConfig();
+  if (config.paths.execution.empty()) {
+    LOG(ERROR) << "browseros: Failed to resolve execution directory";
+    Stop();
+    return;
+  }
+
+  LOG(INFO) << "browseros: Launching " << config.log_name << " - "
+            << config.DebugString();
+  if (launched_installation_ && launched_installation_->version.IsValid()) {
+    LOG(INFO) << "browseros: Selected "
+              << (launched_installation_->downloaded ? "downloaded" : "bundled")
+              << " server " << launched_installation_->version.GetString();
+  }
+  launch_in_progress_ = true;
+  ++process_generation_;
+  health_check_in_progress_ = false;
+  ClearRunningInstallation();
+  auto controller = process_controller_;
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_BLOCKING},
+      base::BindOnce(
+          [](std::shared_ptr<ProcessController> controller,
+             ServerLaunchConfig config) {
+            auto pending = std::make_unique<PendingServerLaunch>();
+            pending->result = controller->Launch(config);
+            return pending;
+          },
+          controller, config),
+      base::BindOnce(
+          [](base::WeakPtr<BrowserOSServerManager> manager,
+             std::unique_ptr<PendingServerLaunch> pending) {
+            if (manager) {
+              manager->OnProcessLaunched(std::move(pending->result));
+            }
+          },
+          process_weak_factory_.GetWeakPtr()));
+}
+
+void BrowserOSServerManager::OnProcessLaunched(LaunchResult result) {
+  launch_in_progress_ = false;
+  process_ = std::move(result.process);
+  if (!started_) {
+    TerminateBrowserOSProcess(
+        base::BindOnce(&BrowserOSServerManager::FinishStop,
+                       process_weak_factory_.GetWeakPtr()));
+    return;
+  }
+
+  if (result.used_fallback && version_store_ && launched_installation_) {
+    if (launched_installation_->downloaded) {
+      version_store_->RejectVersion(launched_installation_->version);
+    }
+    launched_installation_ = version_store_->GetBundled();
+    CompleteUpdate(false);
+  }
+
+  if (!process_.IsValid()) {
+    LOG(ERROR) << "browseros: Failed to launch "
+               << GetManagedServerDescriptor().log_name;
+    InvalidateDownloadedServer();
+    return;
+  }
+
+  is_running_ = true;
+  consecutive_health_failures_ = 0;
+  last_launch_time_ = base::TimeTicks::Now();
+  LOG(INFO) << "browseros: " << GetManagedServerDescriptor().log_name
+            << " started with PID: " << process_.Pid();
+
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
+    }
+  }
+
+  health_check_timer_.Start(FROM_HERE, kHealthCheckInterval, this,
+                            &BrowserOSServerManager::CheckServerHealth);
+  process_check_timer_.Start(FROM_HERE, kProcessCheckInterval, this,
+                             &BrowserOSServerManager::CheckProcessStatus);
+  // A launched PID is not yet an activated server. Bound startup separately
+  // from normal health monitoring, and only complete an OTA after HTTP health.
+  startup_health_deadline_timer_.Start(
+      FROM_HERE, kStartupGracePeriod,
+      base::BindOnce(&BrowserOSServerManager::OnStartupHealthTimeout,
+                     weak_factory_.GetWeakPtr(), process_generation_));
+  CheckServerHealth();
+}
+
+void BrowserOSServerManager::StartUpdater() {
+  if (updater_started_ || !started_) {
+    return;
+  }
+  const ManagedServerDescriptor& descriptor = GetManagedServerDescriptor();
+  if (!updater_) {
+    if (base::CommandLine::ForCurrentProcess()->HasSwitch(
+            browseros::kDisableServerUpdater) ||
+        !descriptor.enable_updater) {
+      return;
+    }
+    updater_ = std::make_unique<browseros_server::BrowserOSServerUpdater>(this);
+  }
+  updater_started_ = true;
+  updater_->Start();
+}
+
+void BrowserOSServerManager::CompleteUpdate(bool success) {
+  is_updating_ = false;
+  if (update_complete_callback_) {
+    std::move(update_complete_callback_).Run(success);
+  }
+}
+
+void BrowserOSServerManager::TerminateBrowserOSProcess(
+    base::OnceCallback<void()> callback) {
+  // Stop supersedes a queued restart but shares the same termination. Never
+  // launch a replacement or release the profile lock while that kill is
+  // pending.
+  termination_callback_ = std::move(callback);
+  if (termination_in_progress_) {
+    return;
+  }
+  readiness_loader_.reset();
+  health_check_timer_.Stop();
+  process_check_timer_.Stop();
+  startup_health_retry_timer_.Stop();
+  startup_health_deadline_timer_.Stop();
+  health_check_in_progress_ = false;
+  ++process_generation_;
+  ClearRunningInstallation();
+  if (!process_.IsValid()) {
+    std::move(termination_callback_).Run();
+    return;
+  }
+
+  termination_in_progress_ = true;
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE,
+      {base::MayBlock(), base::WithBaseSyncPrimitives(),
+       base::TaskPriority::USER_BLOCKING,
+       base::TaskShutdownBehavior::BLOCK_SHUTDOWN},
+      base::BindOnce(
+          [](std::shared_ptr<ProcessController> controller,
+             base::Process process) {
+            int exit_code = 0;
+#if BUILDFLAG(IS_POSIX)
+            if (kill(process.Pid(), SIGTERM) != 0) {
+              return errno == ESRCH;
+            }
+            // Unlike orphan recovery, this is our child. waitpid must reap it:
+            // kill(pid, 0) alone treats an exited zombie as a live server.
+            if (controller->WaitForExitWithTimeout(&process, base::Seconds(5),
+                                                   &exit_code)) {
+              return true;
+            }
+#endif
+            controller->Terminate(&process, false);
+            return controller->WaitForExitWithTimeout(
+                &process, base::Seconds(5), &exit_code);
+          },
+          process_controller_, process_.Duplicate()),
+      base::BindOnce(&BrowserOSServerManager::OnTerminateProcessComplete,
+                     process_weak_factory_.GetWeakPtr()));
+}
+
+void BrowserOSServerManager::OnTerminateProcessComplete(bool killed) {
+  termination_in_progress_ = false;
+  if (!killed) {
+    LOG(ERROR) << "browseros: Managed server termination failed; "
+                  "refusing to launch a second server";
+    termination_callback_.Reset();
+    is_restarting_ = false;
+    CompleteUpdate(false);
+    return;
+  }
+  process_.Close();
+  if (termination_callback_) {
+    std::move(termination_callback_).Run();
+  }
+}
+
+void BrowserOSServerManager::OnProcessExited(int exit_code) {
+  if (!started_ || termination_in_progress_) {
+    return;
+  }
+  LOG(INFO) << "browseros: Managed server exited with code " << exit_code;
+  const bool was_starting = !running_installation_.has_value();
+  const base::TimeDelta uptime = base::TimeTicks::Now() - last_launch_time_;
+  health_check_timer_.Stop();
+  process_check_timer_.Stop();
+  startup_health_retry_timer_.Stop();
+  startup_health_deadline_timer_.Stop();
+  readiness_loader_.reset();
+  health_check_in_progress_ = false;
+  ++process_generation_;
+  process_.Close();
+  ClearRunningInstallation();
+
+  if (exit_code == kExitCodeSuccess && !was_starting) {
+    is_restarting_ = false;
+    return;
+  }
+  if (exit_code == kExitCodePortConflict) {
+    advance_port_on_restart_ = true;
+  }
+  if (uptime < kStartupGracePeriod) {
+    ++consecutive_startup_failures_;
+  } else {
+    consecutive_startup_failures_ = 0;
+  }
+
+  // A binary can pass --version yet fail during real initialization. Reject a
+  // failed first launch immediately, but retry port conflicts on another port.
+  if ((was_starting && exit_code != kExitCodePortConflict) ||
+      consecutive_startup_failures_ >= kMaxStartupFailures) {
+    InvalidateDownloadedServer();
+    return;
+  }
+
+  is_restarting_ = false;
+  RestartBrowserOSProcess();
+}
+
+void BrowserOSServerManager::CheckServerHealth() {
+  if (!is_running_ || health_check_in_progress_ || termination_in_progress_) {
+    return;
+  }
+  health_check_in_progress_ = true;
+  const uint64_t generation = process_generation_;
+  health_checker_->CheckHealth(
+      ports_.server, std::string(GetManagedServerDescriptor().health_path),
+      base::BindOnce(
+          [](base::WeakPtr<BrowserOSServerManager> manager, uint64_t generation,
+             bool success) {
+            // A late response from the old port/process must not mark its
+            // replacement healthy or complete the replacement's activation.
+            if (manager && generation == manager->process_generation_) {
+              manager->health_check_in_progress_ = false;
+              manager->OnHealthCheckComplete(success);
+            }
+          },
+          weak_factory_.GetWeakPtr(), generation));
+}
+
+void BrowserOSServerManager::CheckProcessStatus() {
+  if (!is_running_ || !process_.IsValid() || termination_in_progress_) {
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
+  if (!is_running_ || termination_in_progress_) {
+    return;
+  }
+  if (success) {
+    consecutive_health_failures_ = 0;
+    if (!running_installation_ && launched_installation_) {
+      running_installation_ = launched_installation_;
+      startup_health_retry_timer_.Stop();
+      startup_health_deadline_timer_.Stop();
+      is_restarting_ = false;
+      if (local_state_) {
+        const base::Version version = GetRunningVersion();
+        local_state_->SetString(browseros_server::kServerVersion,
+                                version.IsValid() ? version.GetString() : "");
+        local_state_->SetBoolean(browseros_server::kRestartServerRequested,
+                                 false);
+      }
+      LOG(INFO) << "browseros: Confirmed healthy server "
+                << (GetRunningVersion().IsValid()
+                        ? GetRunningVersion().GetString()
+                        : "(unknown version)");
+      CompleteUpdate(true);
+      StartUpdater();
+      if (updater_) {
+        updater_->OnServerActivated();
+      }
+      MaybeActivateReadyVersion();
+    }
+    return;
+  }
+
+  if (!running_installation_ && launched_installation_) {
+    // An independent deadline also covers hung HTTP requests; this short retry
+    // allows normal server startup latency without publishing an active
+    // version.
+    startup_health_retry_timer_.Start(
+        FROM_HERE, kStartupHealthRetryInterval, this,
+        &BrowserOSServerManager::CheckServerHealth);
+    return;
+  }
+
+  if (++consecutive_health_failures_ >= kMaxConsecutiveHealthCheckFailures) {
+    consecutive_health_failures_ = 0;
+    if (launched_installation_ && launched_installation_->downloaded) {
+      InvalidateDownloadedServer();
+    } else {
+      RestartBrowserOSProcess();
+    }
+  }
+}
+
+void BrowserOSServerManager::OnStartupHealthTimeout(uint64_t generation) {
+  if (!started_ || generation != process_generation_ || running_installation_) {
+    return;
+  }
+  LOG(ERROR)
+      << "browseros: Server did not become healthy before startup deadline";
+  InvalidateDownloadedServer();
+}
+
+void BrowserOSServerManager::InvalidateDownloadedServer() {
+  if (!started_) {
+    return;
+  }
+  if (!launched_installation_ || !launched_installation_->downloaded ||
+      !version_store_) {
+    LOG(ERROR) << "browseros: Bundled server unavailable; stopping sidecar";
+    CompleteUpdate(false);
+    Stop();
+    return;
+  }
+
+  const base::Version rejected = launched_installation_->version;
+  LOG(WARNING) << "browseros: Rejecting failed server " << rejected.GetString();
+  is_restarting_ = true;
+  TerminateBrowserOSProcess(
+      base::BindOnce(&BrowserOSServerManager::OnRejectedProcessStopped,
+                     weak_factory_.GetWeakPtr(), rejected));
+}
+
+void BrowserOSServerManager::OnRejectedProcessStopped(
+    const base::Version& version) {
+  if (!started_ || !version_store_) {
+    return;
+  }
+  version_store_->RejectVersion(version);
+  next_installation_.reset();
+  CompleteUpdate(false);
+  consecutive_startup_failures_ = 0;
+  // Usually this selects the bundle; a newer ready download that arrived while
+  // the old process failed is preserved and remains eligible for this launch.
+  ContinueRestartAfterTerminate();
+}
+
+void BrowserOSServerManager::RestartBrowserOSProcess() {
+  LOG(INFO) << "browseros: Restarting "
+            << GetManagedServerDescriptor().log_name;
+
+  if (!started_ || !version_store_ || !version_store_->initialized() ||
+      launch_in_progress_ || is_restarting_ || is_updating_) {
+    LOG(INFO) << "browseros: Restart already in progress or not initialized";
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
+  base::CommandLine* cl = base::CommandLine::ForCurrentProcess();
+  std::set<int> assigned;
+  assigned.insert(ports_.cdp);
+  assigned.insert(ports_.proxy);
+  assigned.insert(proxy_https_port_);
+
+  const int previous_server_port = ports_.server;
+  if (!cl->HasSwitch(browseros::kServerPort)) {
+    const bool advance_port = advance_port_on_restart_;
+    const int starting_port =
+        advance_port ? previous_server_port + 1 : previous_server_port;
+    ports_.server = FindAvailableServerPort(starting_port, assigned,
+                                            /*allow_reuse=*/!advance_port);
+  }
+  advance_port_on_restart_ = false;
+  assigned.insert(ports_.server);
+
+  if (ports_.server == previous_server_port) {
+    LOG(INFO) << "browseros: Restart keeping server port - "
+              << ports_.DebugString();
+  } else {
+    LOG(INFO) << "browseros: Restart moved server port from "
+              << previous_server_port << " to " << ports_.server << " - "
+              << ports_.DebugString();
+  }
+
+  SavePortsToPrefs();
+  LaunchBrowserOSProcess();
+}
+
+void BrowserOSServerManager::RestartServerForUpdate(
+    UpdateCompleteCallback callback) {
+  LOG(INFO) << "browseros: Restarting server for OTA update";
+
+  if (!started_ || launch_in_progress_ || termination_in_progress_ ||
+      is_restarting_ || is_updating_) {
+    LOG(WARNING) << "browseros: Restart already in progress, deferring update";
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
+  ContinueRestartAfterTerminate();
+}
+
+ServerVersionStore& BrowserOSServerManager::GetVersionStore() {
+  CHECK(version_store_);
+  return *version_store_;
+}
+
+base::Version BrowserOSServerManager::GetRunningVersion() const {
+  return running_installation_ ? running_installation_->version
+                               : base::Version();
+}
+
+void BrowserOSServerManager::MaybeActivateReadyVersion() {
+  if (!started_ || !running_installation_ || !is_running_ || is_restarting_ ||
+      is_updating_ || launch_in_progress_ || termination_in_progress_ ||
+      readiness_loader_ || !version_store_ || !version_store_->initialized()) {
+    return;
+  }
+  const ServerInstallation target = version_store_->GetBestAvailable();
+  const base::Version running = GetRunningVersion();
+  if (!target.downloaded || (running.IsValid() && target.version <= running)) {
+    return;
+  }
+
+  const std::string readiness_path(
+      GetManagedServerDescriptor().updater.readiness_path);
+  if (readiness_path.empty()) {
+    ActivateInstallation(target);
+    return;
+  }
+
+  auto request = std::make_unique<network::ResourceRequest>();
+  request->url =
+      GURL("http://127.0.0.1:" + base::NumberToString(ports_.server) +
+           readiness_path);
+  request->credentials_mode = network::mojom::CredentialsMode::kOmit;
+  readiness_loader_ = network::SimpleURLLoader::Create(
+      std::move(request), GetStatusTrafficAnnotation());
+  readiness_loader_->SetTimeoutDuration(browseros_server::kStatusCheckTimeout);
+  readiness_loader_->DownloadToString(
+      g_browser_process->system_network_context_manager()
+          ->GetURLLoaderFactory(),
+      base::BindOnce(&BrowserOSServerManager::OnReadinessChecked,
+                     weak_factory_.GetWeakPtr(), target, process_generation_),
+      4096);
+}
+
+void BrowserOSServerManager::OnReadinessChecked(
+    ServerInstallation installation,
+    uint64_t generation,
+    std::optional<std::string> response) {
+  readiness_loader_.reset();
+  if (!started_ || generation != process_generation_ || !version_store_ ||
+      !running_installation_) {
+    return;
+  }
+  // Unavailable/malformed readiness is not consent to interrupt work. Keep the
+  // ready installation and let the independent activation timer try again.
+  const std::optional<base::Value> status =
+      response ? base::JSONReader::Read(*response, base::JSON_PARSE_RFC)
+               : std::nullopt;
+  if (!status || !status->is_dict() ||
+      !status->GetDict().FindBool("can_update").value_or(false)) {
+    LOG(INFO) << "browseros: Server busy or readiness unknown; keeping version "
+              << installation.version.GetString() << " ready for retry";
+    return;
+  }
+  const base::Version best = version_store_->GetBestAvailable().version;
+  if (!best.IsValid() || best != installation.version) {
+    MaybeActivateReadyVersion();
+    return;
+  }
+  ActivateInstallation(installation);
+}
+
+void BrowserOSServerManager::ActivateInstallation(
+    const ServerInstallation& installation) {
+  if (!started_ || is_restarting_ || is_updating_ || launch_in_progress_ ||
+      termination_in_progress_) {
+    return;
+  }
+  next_installation_ = installation;
+  const base::Version old_version = GetRunningVersion();
+  RestartServerForUpdate(base::BindOnce(
+      &BrowserOSServerManager::OnReadyVersionActivated,
+      weak_factory_.GetWeakPtr(), old_version, installation.version));
+}
+
+void BrowserOSServerManager::OnReadyVersionActivated(
+    const base::Version& old_version,
+    const base::Version& new_version,
+    bool success) {
+  base::DictValue properties;
+  properties.Set("old_version",
+                 old_version.IsValid() ? old_version.GetString() : "unknown");
+  properties.Set("new_version", new_version.GetString());
+  if (!success) {
+    properties.Set("stage", "activation");
+    properties.Set("error", "Prepared server failed to become healthy");
+  }
+  browseros_metrics::BrowserOSMetrics::Log(
+      success ? "server.ota.success" : "server.ota.error",
+      std::move(properties));
+}
+
+void BrowserOSServerManager::OnAllowRemoteInMCPChanged() {
+  if (!is_running_ || !local_state_) {
+    return;
+  }
+
+  bool new_value =
+      local_state_->GetBoolean(browseros_server::kAllowRemoteInMCP);
+
+  if (new_value != allow_remote_in_mcp_) {
+    LOG(INFO) << "browseros: allow_remote_in_mcp preference changed from "
+              << (allow_remote_in_mcp_ ? "true" : "false") << " to "
+              << (new_value ? "true" : "false") << ", restarting server...";
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
+void BrowserOSServerManager::OnProxyPortChanged() {
+  if (!is_running_ || !local_state_) {
+    return;
+  }
+  if (base::CommandLine::ForCurrentProcess()->HasSwitch(
+          browseros::kProxyPort)) {
+    LOG(INFO) << "browseros: Ignoring proxy_port pref change "
+              << "(CLI --browseros-proxy-port overrides)";
+    return;
+  }
+
+  int new_port = local_state_->GetInteger(browseros_server::kProxyPort);
+  if (new_port <= 0) {
+    new_port = browseros_server::kDefaultProxyPort;
+  }
+  if (new_port == ports_.proxy) {
+    return;
+  }
+  if (!net::IsPortValid(new_port)) {
+    LOG(WARNING) << "browseros: Invalid proxy port " << new_port
+                 << " (must be 1-65535), ignoring pref change";
+    return;
+  }
+  if (new_port == ports_.cdp || new_port == ports_.server) {
+    LOG(WARNING) << "browseros: Proxy port " << new_port
+                 << " collides with another bound port, ignoring pref change";
+    return;
+  }
+
+  LOG(INFO) << "browseros: proxy_port preference changed from " << ports_.proxy
+            << " to " << new_port << ", rebinding proxy and restarting server";
+  ports_.proxy = new_port;
+  SavePortsToPrefs();
+  StopProxy();
+  StartProxy();
+  RestartBrowserOSProcess();
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
+  return exe_dir.Append(GetManagedServerDescriptor().bundle_dir)
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
+  base::FilePath exec_dir =
+      user_data_dir.Append(FILE_PATH_LITERAL(".browseros"));
+
+  base::ScopedAllowBlocking allow_blocking;
+  if (!base::PathExists(exec_dir)) {
+    if (!base::CreateDirectory(exec_dir)) {
+      LOG(ERROR) << "browseros: Failed to create execution directory: "
+                 << exec_dir;
+      return base::FilePath();
+    }
+  }
+
+  LOG(INFO) << "browseros: Using execution directory: " << exec_dir;
+  return exec_dir;
+}
+
+base::FilePath BrowserOSServerManager::GetBrowserOSServerExecutablePath()
+    const {
+  base::FilePath browseros_exe =
+      GetBrowserOSServerResourcesPath()
+          .Append(FILE_PATH_LITERAL("bin"))
+          .Append(GetManagedServerDescriptor().binary_name);
+
+#if BUILDFLAG(IS_WIN)
+  browseros_exe = browseros_exe.AddExtension(FILE_PATH_LITERAL(".exe"));
+#endif
+
+  return browseros_exe;
+}
+
+}  // namespace browseros
