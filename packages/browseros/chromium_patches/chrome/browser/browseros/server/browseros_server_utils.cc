diff --git a/chrome/browser/browseros/server/browseros_server_utils.cc b/chrome/browser/browseros/server/browseros_server_utils.cc
new file mode 100644
index 0000000000000..8fbf4571c47c5
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_utils.cc
@@ -0,0 +1,503 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_server_utils.h"
+
+#include <optional>
+
+#include "base/command_line.h"
+#include "base/files/file_util.h"
+#include "base/json/json_reader.h"
+#include "base/json/json_writer.h"
+#include "base/logging.h"
+#include "base/path_service.h"
+#include "base/process/process.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/strings/string_split.h"
+#include "base/threading/platform_thread.h"
+#include "build/build_config.h"
+#include "chrome/browser/browseros/core/browseros_switches.h"
+#include "chrome/common/chrome_paths.h"
+#include "components/version_info/version_info.h"
+#include "net/base/ip_address.h"
+#include "net/base/ip_endpoint.h"
+#include "net/base/net_errors.h"
+#include "net/base/port_util.h"
+#include "net/log/net_log_source.h"
+#include "net/socket/tcp_socket.h"
+
+#if BUILDFLAG(IS_POSIX)
+#include <signal.h>
+#include <sys/types.h>
+#endif
+
+#if BUILDFLAG(IS_MAC)
+#include <libproc.h>
+#include <sys/proc_info.h>
+#endif
+
+#if BUILDFLAG(IS_LINUX)
+#include <sys/sysinfo.h>
+
+#include "base/files/file_path.h"
+#endif
+
+#if BUILDFLAG(IS_WIN)
+#include <windows.h>
+
+#include "base/win/scoped_handle.h"
+#endif
+
+namespace browseros::server_utils {
+
+namespace {
+
+constexpr int kMaxPortAttempts = 100;
+constexpr int kMaxPort = 65535;
+
+constexpr base::FilePath::CharType kStateFileName[] =
+    FILE_PATH_LITERAL("server.state");
+constexpr base::FilePath::CharType kLockFileName[] =
+    FILE_PATH_LITERAL("server.lock");
+
+}  // namespace
+
+// =============================================================================
+// Port Management
+// =============================================================================
+
+int FindAvailablePort(int starting_port, const std::set<int>& excluded) {
+  LOG(INFO) << "browseros: Finding port starting from " << starting_port;
+
+  for (int i = 0; i < kMaxPortAttempts; i++) {
+    int port_to_try = starting_port + i;
+
+    if (port_to_try > kMaxPort) {
+      break;
+    }
+
+    if (excluded.count(port_to_try) > 0) {
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
+               << kMaxPortAttempts << " attempts, using " << starting_port
+               << " anyway";
+  return starting_port;
+}
+
+bool IsPortAvailable(int port) {
+  if (!net::IsPortValid(port) || port == 0) {
+    return false;
+  }
+
+  if (net::IsWellKnownPort(port)) {
+    return false;
+  }
+
+  if (!net::IsPortAllowedForScheme(port, "http")) {
+    return false;
+  }
+
+  // Use TCPSocket directly instead of TCPServerSocket to avoid SO_REUSEADDR.
+  // TCPServerSocket::Listen() calls SetDefaultOptionsForServer() which sets
+  // SO_REUSEADDR, allowing bind to succeed even when another socket is bound
+  // to 0.0.0.0 (especially on macOS). By using TCPSocket directly and NOT
+  // calling SetDefaultOptionsForServer(), we get accurate port availability.
+
+  // Try binding to IPv4 localhost
+  auto socket = net::TCPSocket::Create(nullptr, nullptr, net::NetLogSource());
+  int result = socket->Open(net::ADDRESS_FAMILY_IPV4);
+  if (result != net::OK) {
+    return false;
+  }
+  result = socket->Bind(net::IPEndPoint(net::IPAddress::IPv4Localhost(), port));
+  socket->Close();
+  if (result != net::OK) {
+    return false;
+  }
+
+  // Try binding to IPv6 localhost
+  auto socket6 = net::TCPSocket::Create(nullptr, nullptr, net::NetLogSource());
+  result = socket6->Open(net::ADDRESS_FAMILY_IPV6);
+  if (result != net::OK) {
+    return false;
+  }
+  result =
+      socket6->Bind(net::IPEndPoint(net::IPAddress::IPv6Localhost(), port));
+  socket6->Close();
+  if (result != net::OK) {
+    return false;
+  }
+
+  return true;
+}
+
+// =============================================================================
+// Path Utilities
+// =============================================================================
+
+base::FilePath GetExecutionDir() {
+  base::FilePath user_data_dir;
+  if (!base::PathService::Get(chrome::DIR_USER_DATA, &user_data_dir)) {
+    LOG(ERROR) << "browseros: Failed to resolve DIR_USER_DATA path";
+    return base::FilePath();
+  }
+
+  base::FilePath exec_dir =
+      user_data_dir.Append(FILE_PATH_LITERAL(".browseros"));
+
+  if (!base::PathExists(exec_dir)) {
+    if (!base::CreateDirectory(exec_dir)) {
+      LOG(ERROR) << "browseros: Failed to create execution directory: "
+                 << exec_dir;
+      return base::FilePath();
+    }
+  }
+
+  return exec_dir;
+}
+
+base::FilePath GetBundledResourcesPath() {
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
+  // Navigate to Resources folder in the app bundle
+  // Chrome.app/Contents/MacOS -> Chrome.app/Contents/Resources
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
+base::FilePath GetBundledExecutablePath() {
+  base::FilePath browseros_exe =
+      GetBundledResourcesPath()
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
+base::FilePath GetLockFilePath() {
+  base::FilePath exec_dir = GetExecutionDir();
+  if (exec_dir.empty()) {
+    return base::FilePath();
+  }
+  return exec_dir.Append(kLockFileName);
+}
+
+base::FilePath GetStateFilePath() {
+  base::FilePath exec_dir = GetExecutionDir();
+  if (exec_dir.empty()) {
+    return base::FilePath();
+  }
+  return exec_dir.Append(kStateFileName);
+}
+
+// =============================================================================
+// State File (Orphan Recovery)
+// =============================================================================
+
+std::optional<ServerState> ReadStateFile() {
+  base::FilePath state_path = GetStateFilePath();
+  if (state_path.empty()) {
+    return std::nullopt;
+  }
+
+  std::string contents;
+  if (!base::ReadFileToString(state_path, &contents)) {
+    return std::nullopt;
+  }
+
+  std::optional<base::Value> parsed = base::JSONReader::Read(contents);
+  if (!parsed || !parsed->is_dict()) {
+    LOG(WARNING) << "browseros: Invalid state file format";
+    return std::nullopt;
+  }
+
+  const base::Value::Dict& dict = parsed->GetDict();
+  std::optional<int> pid = dict.FindInt("pid");
+  std::optional<double> creation_time = dict.FindDouble("creation_time");
+
+  if (!pid || !creation_time) {
+    LOG(WARNING) << "browseros: State file missing required fields";
+    return std::nullopt;
+  }
+
+  ServerState state;
+  state.pid = static_cast<base::ProcessId>(*pid);
+  state.creation_time = static_cast<int64_t>(*creation_time);
+
+  LOG(INFO) << "browseros: Read state file - PID: " << state.pid
+            << ", creation_time: " << state.creation_time;
+  return state;
+}
+
+bool WriteStateFile(const ServerState& state) {
+  base::FilePath state_path = GetStateFilePath();
+  if (state_path.empty()) {
+    return false;
+  }
+
+  base::Value::Dict dict;
+  dict.Set("pid", static_cast<int>(state.pid));
+  dict.Set("creation_time", static_cast<double>(state.creation_time));
+
+  std::optional<std::string> json_output = base::WriteJson(dict);
+  if (!json_output.has_value()) {
+    LOG(ERROR) << "browseros: Failed to serialize state to JSON";
+    return false;
+  }
+
+  if (!base::WriteFile(state_path, json_output.value())) {
+    LOG(ERROR) << "browseros: Failed to write state file: " << state_path;
+    return false;
+  }
+
+  LOG(INFO) << "browseros: Wrote state file - PID: " << state.pid
+            << ", creation_time: " << state.creation_time;
+  return true;
+}
+
+bool DeleteStateFile() {
+  base::FilePath state_path = GetStateFilePath();
+  if (state_path.empty()) {
+    return true;  // No state file path = nothing to delete
+  }
+
+  if (!base::PathExists(state_path)) {
+    return true;  // Already gone
+  }
+
+  if (!base::DeleteFile(state_path)) {
+    LOG(WARNING) << "browseros: Failed to delete state file: " << state_path;
+    return false;
+  }
+
+  LOG(INFO) << "browseros: Deleted state file";
+  return true;
+}
+
+// =============================================================================
+// Process Utilities
+// =============================================================================
+
+std::optional<int64_t> GetProcessCreationTime(base::ProcessId pid) {
+#if BUILDFLAG(IS_MAC)
+  struct proc_bsdinfo info;
+  int size = proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, sizeof(info));
+  if (size != sizeof(info)) {
+    return std::nullopt;
+  }
+  // pbi_start_tvsec is seconds, pbi_start_tvusec is microseconds
+  return static_cast<int64_t>(info.pbi_start_tvsec) * 1000 +
+         static_cast<int64_t>(info.pbi_start_tvusec) / 1000;
+
+#elif BUILDFLAG(IS_LINUX)
+  // Read /proc/{pid}/stat to get starttime (field 22)
+  std::string stat_path = "/proc/" + base::NumberToString(pid) + "/stat";
+  std::string contents;
+  if (!base::ReadFileToString(base::FilePath(stat_path), &contents)) {
+    return std::nullopt;
+  }
+
+  // Format: pid (comm) state ppid ... starttime ...
+  // Find the closing paren of comm to handle spaces in process name
+  size_t comm_end = contents.rfind(')');
+  if (comm_end == std::string::npos) {
+    return std::nullopt;
+  }
+
+  // Fields after comm start at index 2 (0-indexed: pid=0, comm=1)
+  // starttime is field 21 (0-indexed), so it's the 19th field after comm
+  std::string remainder = contents.substr(comm_end + 2);  // Skip ") "
+  std::vector<std::string> fields = base::SplitString(
+      remainder, " ", base::KEEP_WHITESPACE, base::SPLIT_WANT_ALL);
+
+  // starttime is at index 19 in the fields after (comm)
+  if (fields.size() < 20) {
+    return std::nullopt;
+  }
+
+  int64_t starttime_jiffies;
+  if (!base::StringToInt64(fields[19], &starttime_jiffies)) {
+    return std::nullopt;
+  }
+
+  // Get system boot time and clock ticks
+  struct sysinfo si;
+  if (sysinfo(&si) != 0) {
+    return std::nullopt;
+  }
+
+  long ticks_per_sec = sysconf(_SC_CLK_TCK);
+  if (ticks_per_sec <= 0) {
+    return std::nullopt;
+  }
+
+  // Calculate: boot_time + (starttime_jiffies / ticks_per_sec) in milliseconds
+  int64_t boot_time_ms =
+      (time(nullptr) - si.uptime) * 1000;  // Approximate boot time
+  int64_t start_offset_ms = (starttime_jiffies * 1000) / ticks_per_sec;
+
+  return boot_time_ms + start_offset_ms;
+
+#elif BUILDFLAG(IS_WIN)
+  base::win::ScopedHandle handle(
+      OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid));
+  if (!handle.IsValid()) {
+    return std::nullopt;
+  }
+
+  FILETIME creation, exit, kernel, user;
+  if (!GetProcessTimes(handle.Get(), &creation, &exit, &kernel, &user)) {
+    return std::nullopt;
+  }
+
+  // Convert FILETIME to milliseconds since epoch
+  // FILETIME is 100-nanosecond intervals since January 1, 1601
+  // Subtract the difference between 1601 and 1970 (epoch)
+  ULARGE_INTEGER uli;
+  uli.LowPart = creation.dwLowDateTime;
+  uli.HighPart = creation.dwHighDateTime;
+  return static_cast<int64_t>((uli.QuadPart - 116444736000000000ULL) / 10000);
+
+#else
+  return std::nullopt;
+#endif
+}
+
+bool ProcessExists(base::ProcessId pid) {
+#if BUILDFLAG(IS_POSIX)
+  // kill with signal 0 checks if process exists without sending a signal
+  return kill(pid, 0) == 0;
+#elif BUILDFLAG(IS_WIN)
+  base::win::ScopedHandle handle(
+      OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid));
+  if (!handle.IsValid()) {
+    return false;
+  }
+  DWORD exit_code;
+  if (!GetExitCodeProcess(handle.Get(), &exit_code)) {
+    return false;
+  }
+  return exit_code == STILL_ACTIVE;
+#else
+  return false;
+#endif
+}
+
+bool KillProcess(base::ProcessId pid, base::TimeDelta graceful_timeout) {
+#if BUILDFLAG(IS_POSIX)
+  // First try SIGTERM for graceful shutdown
+  if (kill(pid, SIGTERM) != 0) {
+    if (errno == ESRCH) {
+      return true;  // Process already gone
+    }
+    PLOG(WARNING) << "browseros: Failed to send SIGTERM to PID " << pid;
+    return false;
+  }
+
+  // Wait for graceful shutdown
+  base::TimeTicks deadline = base::TimeTicks::Now() + graceful_timeout;
+  while (base::TimeTicks::Now() < deadline) {
+    if (!ProcessExists(pid)) {
+      LOG(INFO) << "browseros: Process " << pid
+                << " terminated gracefully after SIGTERM";
+      return true;
+    }
+    base::PlatformThread::Sleep(base::Milliseconds(100));
+  }
+
+  // Still running, send SIGKILL
+  LOG(WARNING) << "browseros: Process " << pid
+               << " did not terminate after SIGTERM, sending SIGKILL";
+  if (kill(pid, SIGKILL) != 0) {
+    if (errno == ESRCH) {
+      return true;  // Gone between checks
+    }
+    PLOG(ERROR) << "browseros: Failed to send SIGKILL to PID " << pid;
+    return false;
+  }
+
+  // Wait briefly for SIGKILL to take effect
+  base::PlatformThread::Sleep(base::Milliseconds(500));
+  return !ProcessExists(pid);
+
+#elif BUILDFLAG(IS_WIN)
+  base::win::ScopedHandle handle(OpenProcess(PROCESS_TERMINATE, FALSE, pid));
+  if (!handle.IsValid()) {
+    DWORD error = GetLastError();
+    if (error == ERROR_INVALID_PARAMETER) {
+      return true;  // Process doesn't exist
+    }
+    LOG(ERROR) << "browseros: Failed to open process " << pid
+               << " for termination, error: " << error;
+    return false;
+  }
+
+  if (!TerminateProcess(handle.Get(), 1)) {
+    DWORD error = GetLastError();
+    LOG(ERROR) << "browseros: Failed to terminate process " << pid
+               << ", error: " << error;
+    return false;
+  }
+
+  // Wait for process to exit
+  DWORD wait_result =
+      WaitForSingleObject(handle.Get(),
+                          static_cast<DWORD>(graceful_timeout.InMilliseconds()));
+  return wait_result == WAIT_OBJECT_0;
+
+#else
+  return false;
+#endif
+}
+
+}  // namespace browseros::server_utils
