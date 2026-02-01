diff --git a/chrome/browser/browseros/server/process_controller_impl.cc b/chrome/browser/browseros/server/process_controller_impl.cc
new file mode 100644
index 0000000000000..a398856f672fd
--- /dev/null
+++ b/chrome/browser/browseros/server/process_controller_impl.cc
@@ -0,0 +1,210 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/process_controller_impl.h"
+
+#include <optional>
+
+#include "chrome/browser/browseros/server/browseros_server_utils.h"
+
+#include "base/files/file_util.h"
+#include "base/json/json_writer.h"
+#include "base/logging.h"
+#include "base/process/launch.h"
+#include "base/strings/string_number_conversions.h"
+#include "build/build_config.h"
+
+#if BUILDFLAG(IS_POSIX)
+#include <signal.h>
+#endif
+
+namespace browseros {
+
+namespace {
+
+constexpr base::FilePath::CharType kConfigFileName[] =
+    FILE_PATH_LITERAL("server_config.json");
+
+// Writes the server configuration to a JSON file.
+// Returns the path to the config file on success, empty path on failure.
+// Note: resources_dir is passed separately because it may differ from
+// config.paths.resources when fallback is used.
+base::FilePath WriteConfigJson(const ServerLaunchConfig& config,
+                               const base::FilePath& actual_resources_dir) {
+  base::FilePath config_path = config.paths.execution.Append(kConfigFileName);
+
+  base::Value::Dict root;
+
+  // ports
+  base::Value::Dict ports_dict;
+  ports_dict.Set("cdp", config.ports.cdp);
+  ports_dict.Set("server", config.ports.server);
+  ports_dict.Set("extension", config.ports.extension);
+  root.Set("ports", std::move(ports_dict));
+
+  // directories
+  base::Value::Dict directories;
+  directories.Set("resources", actual_resources_dir.AsUTF8Unsafe());
+  directories.Set("execution", config.paths.execution.AsUTF8Unsafe());
+  root.Set("directories", std::move(directories));
+
+  // flags
+  base::Value::Dict flags;
+  flags.Set("allow_remote_in_mcp", config.allow_remote_in_mcp);
+  root.Set("flags", std::move(flags));
+
+  // instance
+  base::Value::Dict instance;
+  instance.Set("install_id", config.identity.install_id);
+  instance.Set("browseros_version", config.identity.browseros_version);
+  instance.Set("chromium_version", config.identity.chromium_version);
+  root.Set("instance", std::move(instance));
+
+  std::optional<std::string> json_output = base::WriteJson(root);
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
+}  // namespace
+
+ProcessControllerImpl::ProcessControllerImpl() = default;
+
+ProcessControllerImpl::~ProcessControllerImpl() = default;
+
+LaunchResult ProcessControllerImpl::Launch(const ServerLaunchConfig& config) {
+  LaunchResult result;
+  base::FilePath actual_exe_path = config.paths.exe;
+  base::FilePath actual_resources_dir = config.paths.resources;
+
+  // Check if executable exists, fallback to bundled if not
+  if (!base::PathExists(actual_exe_path)) {
+    LOG(WARNING) << "browseros: Binary not found at " << actual_exe_path
+                 << ", falling back to bundled";
+    actual_exe_path = config.paths.fallback_exe;
+    actual_resources_dir = config.paths.fallback_resources;
+    result.used_fallback = true;
+
+    if (!base::PathExists(actual_exe_path)) {
+      LOG(ERROR) << "browseros: Bundled binary also not found at: "
+                 << actual_exe_path;
+      return result;
+    }
+  }
+
+  if (config.paths.execution.empty()) {
+    LOG(ERROR) << "browseros: Execution directory path is empty";
+    return result;
+  }
+
+  // Ensure execution directory exists (blocking I/O)
+  if (!base::CreateDirectory(config.paths.execution)) {
+    LOG(ERROR) << "browseros: Failed to create execution directory at: "
+               << config.paths.execution;
+    return result;
+  }
+
+  // Write configuration to JSON file
+  base::FilePath config_path = WriteConfigJson(config, actual_resources_dir);
+  if (config_path.empty()) {
+    LOG(ERROR) << "browseros: Failed to write config file, aborting launch";
+    return result;
+  }
+
+  // Build command line with --config flag and explicit port args
+  base::CommandLine cmd(actual_exe_path);
+  cmd.AppendSwitchPath("config", config_path);
+  cmd.AppendSwitchASCII("cdp-port", base::NumberToString(config.ports.cdp));
+  cmd.AppendSwitchASCII("server-port", base::NumberToString(config.ports.server));
+  cmd.AppendSwitchASCII("extension-port",
+                        base::NumberToString(config.ports.extension));
+
+  // Set up launch options
+  base::LaunchOptions options;
+#if BUILDFLAG(IS_WIN)
+  options.start_hidden = true;
+#endif
+
+  // Launch the process (blocking I/O)
+  result.process = base::LaunchProcess(cmd, options);
+  return result;
+}
+
+void ProcessControllerImpl::Terminate(base::Process* process, bool wait) {
+  if (!process || !process->IsValid()) {
+    return;
+  }
+
+  LOG(INFO) << "browseros: Terminating process with SIGKILL (PID: "
+            << process->Pid() << ", wait: " << (wait ? "true" : "false") << ")";
+
+#if BUILDFLAG(IS_POSIX)
+  base::ProcessId pid = process->Pid();
+  if (kill(pid, SIGKILL) != 0) {
+    PLOG(ERROR) << "browseros: Failed to send SIGKILL to PID " << pid;
+  } else if (wait) {
+    // Blocking wait - caller must ensure this runs on a thread with MayBlock()
+    int exit_code = 0;
+    if (process->WaitForExit(&exit_code)) {
+      LOG(INFO) << "browseros: Process killed successfully";
+    } else {
+      LOG(WARNING) << "browseros: WaitForExit failed";
+    }
+  } else {
+    LOG(INFO) << "browseros: SIGKILL sent (not waiting for exit)";
+  }
+#else
+  // Windows: Terminate with wait parameter
+  bool terminated = process->Terminate(0, wait);
+  if (terminated) {
+    LOG(INFO) << "browseros: Process terminated successfully";
+  } else {
+    LOG(ERROR) << "browseros: Failed to terminate process";
+  }
+#endif
+}
+
+bool ProcessControllerImpl::WaitForExitWithTimeout(base::Process* process,
+                                                   base::TimeDelta timeout,
+                                                   int* exit_code) {
+  if (!process || !process->IsValid()) {
+    return true;  // No process to wait for
+  }
+
+  LOG(INFO) << "browseros: Waiting for process exit (PID: " << process->Pid()
+            << ", timeout: " << timeout.InSeconds() << "s)";
+
+  bool exited = process->WaitForExitWithTimeout(timeout, exit_code);
+  if (exited) {
+    LOG(INFO) << "browseros: Process exited with code " << *exit_code;
+  } else {
+    LOG(INFO) << "browseros: Process did not exit within timeout";
+  }
+  return exited;
+}
+
+bool ProcessControllerImpl::Exists(base::ProcessId pid) {
+  return server_utils::ProcessExists(pid);
+}
+
+std::optional<int64_t> ProcessControllerImpl::GetCreationTime(
+    base::ProcessId pid) {
+  return server_utils::GetProcessCreationTime(pid);
+}
+
+bool ProcessControllerImpl::Kill(base::ProcessId pid,
+                                 base::TimeDelta graceful_timeout) {
+  return server_utils::KillProcess(pid, graceful_timeout);
+}
+
+}  // namespace browseros
