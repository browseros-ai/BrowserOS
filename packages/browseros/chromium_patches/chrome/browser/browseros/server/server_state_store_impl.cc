diff --git a/chrome/browser/browseros/server/server_state_store_impl.cc b/chrome/browser/browseros/server/server_state_store_impl.cc
new file mode 100644
index 0000000000000000000000000000000000000000..0ae286fc4ab223d375bc8ce267e901b2a47e158d
--- /dev/null
+++ b/chrome/browser/browseros/server/server_state_store_impl.cc
@@ -0,0 +1,323 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/server_state_store_impl.h"
+
+#include "base/check.h"
+#include "base/command_line.h"
+#include "base/files/file.h"
+#include "base/files/file_util.h"
+#include "base/files/important_file_writer.h"
+#include "base/files/scoped_temp_dir.h"
+#include "base/functional/bind.h"
+#include "base/functional/callback_helpers.h"
+#include "base/logging.h"
+#include "base/process/launch.h"
+#include "base/process/process.h"
+#include "base/strings/string_util.h"
+#include "base/task/thread_pool.h"
+#include "build/build_config.h"
+#include "chrome/browser/browseros/server/browseros_server_config.h"
+#include "chrome/browser/browseros/server/browseros_server_constants.h"
+#include "chrome/browser/browseros/server/server_version_store.h"
+
+#if BUILDFLAG(IS_WIN)
+#include <windows.h>
+#else
+#include <signal.h>
+#include <unistd.h>
+#endif
+
+namespace browseros {
+
+namespace {
+
+constexpr base::TimeDelta kVersionProbeTimeout = base::Seconds(10);
+constexpr size_t kMaxVersionOutput = 4096;
+
+// --version starts a separate process, not the serving process. Probe the exact
+// selected path before launch; the manager later confirms that launch's health.
+// A timeout bounds startup work without blocking Chromium's UI sequence.
+base::Version ProbeServerVersion(const base::FilePath& executable) {
+  base::ScopedTempDir temporary_dir;
+  if (!temporary_dir.CreateUniqueTempDir()) {
+    return base::Version();
+  }
+  const base::FilePath output_path =
+      temporary_dir.GetPath().AppendASCII("version");
+  base::File output_file(output_path,
+                         base::File::FLAG_CREATE | base::File::FLAG_WRITE);
+#if BUILDFLAG(IS_WIN)
+  base::File null_file(
+      base::FilePath(FILE_PATH_LITERAL("NUL")),
+      base::File::FLAG_OPEN | base::File::FLAG_READ | base::File::FLAG_WRITE);
+#else
+  base::File null_file(
+      base::FilePath(FILE_PATH_LITERAL("/dev/null")),
+      base::File::FLAG_OPEN | base::File::FLAG_READ | base::File::FLAG_WRITE);
+#endif
+  if (!output_file.IsValid() || !null_file.IsValid()) {
+    return base::Version();
+  }
+
+  base::LaunchOptions options;
+#if BUILDFLAG(IS_WIN)
+  if (!SetHandleInformation(output_file.GetPlatformFile(), HANDLE_FLAG_INHERIT,
+                            HANDLE_FLAG_INHERIT) ||
+      !SetHandleInformation(null_file.GetPlatformFile(), HANDLE_FLAG_INHERIT,
+                            HANDLE_FLAG_INHERIT)) {
+    return base::Version();
+  }
+  options.start_hidden = true;
+  options.stdin_handle = null_file.GetPlatformFile();
+  options.stdout_handle = output_file.GetPlatformFile();
+  options.stderr_handle = null_file.GetPlatformFile();
+  options.handles_to_inherit = {output_file.GetPlatformFile(),
+                                null_file.GetPlatformFile()};
+#else
+  options.fds_to_remap = {{null_file.GetPlatformFile(), STDIN_FILENO},
+                          {output_file.GetPlatformFile(), STDOUT_FILENO},
+                          {null_file.GetPlatformFile(), STDERR_FILENO}};
+#endif
+  base::CommandLine command(executable);
+  command.AppendSwitch("version");
+  base::Process process = base::LaunchProcess(command, options);
+  int exit_code = -1;
+  if (!process.IsValid()) {
+    return base::Version();
+  }
+  if (!process.WaitForExitWithTimeout(kVersionProbeTimeout, &exit_code)) {
+    LOG(WARNING) << "browseros: Version probe timed out: " << executable;
+#if BUILDFLAG(IS_WIN)
+    process.Terminate(-1, false);
+#else
+    kill(process.Pid(), SIGKILL);
+#endif
+    process.WaitForExitWithTimeout(base::Seconds(5), &exit_code);
+    return base::Version();
+  }
+  output_file.Close();
+  std::string output;
+  if (exit_code != 0 || !base::ReadFileToStringWithMaxSize(output_path, &output,
+                                                           kMaxVersionOutput)) {
+    return base::Version();
+  }
+  return base::Version(
+      std::string(base::TrimWhitespaceASCII(output, base::TRIM_ALL)));
+}
+
+base::Version ReadReadyVersion(const base::FilePath& marker) {
+  std::string content;
+  if (!base::ReadFileToStringWithMaxSize(marker, &content, kMaxVersionOutput)) {
+    return base::Version();
+  }
+  return base::Version(
+      std::string(base::TrimWhitespaceASCII(content, base::TRIM_ALL)));
+}
+
+ServerInstallation DownloadedInstallation(const base::FilePath& execution_dir,
+                                          const base::FilePath& binary_name,
+                                          const base::Version& version) {
+  ServerInstallation installation;
+  installation.version = version;
+  installation.resources =
+      execution_dir.AppendASCII(browseros_server::kVersionsDirectoryName)
+          .AppendASCII(version.GetString())
+          .AppendASCII("resources");
+  installation.executable =
+      installation.resources.AppendASCII("bin").Append(binary_name);
+#if BUILDFLAG(IS_WIN)
+  installation.executable =
+      installation.executable.AddExtension(FILE_PATH_LITERAL(".exe"));
+#endif
+  installation.downloaded = true;
+  return installation;
+}
+
+void RemoveRejectedVersion(const base::FilePath& execution_dir,
+                           const base::Version& version) {
+  const base::FilePath marker =
+      execution_dir.AppendASCII(browseros_server::kCurrentVersionFileName);
+  // A newer download may already have replaced this selection. Never erase its
+  // marker while cleaning up a failed older process.
+  const base::Version selected = ReadReadyVersion(marker);
+  if (selected.IsValid() && selected == version) {
+    base::DeleteFile(marker);
+  }
+  base::DeletePathRecursively(
+      execution_dir.AppendASCII(browseros_server::kVersionsDirectoryName)
+          .AppendASCII(version.GetString()));
+}
+
+}  // namespace
+
+ServerStateStoreImpl::ServerStateStoreImpl() = default;
+
+ServerStateStoreImpl::~ServerStateStoreImpl() = default;
+
+std::optional<server_utils::ServerState> ServerStateStoreImpl::Read() {
+  return server_utils::ReadStateFile();
+}
+
+bool ServerStateStoreImpl::Write(const server_utils::ServerState& state) {
+  return server_utils::WriteStateFile(state);
+}
+
+bool ServerStateStoreImpl::Delete() {
+  return server_utils::DeleteStateFile();
+}
+
+ServerVersionStore::ServerVersionStore(
+    const base::FilePath& execution_dir,
+    ServerInstallation bundled,
+    const ManagedServerDescriptor& descriptor)
+    : execution_dir_(descriptor.updater.state_dir.empty()
+                         ? execution_dir
+                         : execution_dir.Append(descriptor.updater.state_dir)),
+      binary_name_(descriptor.binary_name),
+      bundled_(std::move(bundled)),
+      task_runner_(base::ThreadPool::CreateSequencedTaskRunner(
+          {base::MayBlock(), base::WithBaseSyncPrimitives(),
+           base::TaskPriority::USER_BLOCKING,
+           base::TaskShutdownBehavior::BLOCK_SHUTDOWN})) {
+  CHECK(!execution_dir.empty());
+}
+
+ServerVersionStore::~ServerVersionStore() = default;
+
+void ServerVersionStore::Shutdown(base::OnceClosure callback) {
+  weak_factory_.InvalidateWeakPtrs();
+  task_runner_->PostTaskAndReply(FROM_HERE, base::DoNothing(),
+                                 std::move(callback));
+}
+
+void ServerVersionStore::Initialize(base::OnceClosure callback) {
+  using Installations =
+      std::pair<ServerInstallation, std::optional<ServerInstallation>>;
+  task_runner_->PostTaskAndReplyWithResult(
+      FROM_HERE,
+      base::BindOnce(
+          [](base::FilePath execution_dir, base::FilePath binary_name,
+             ServerInstallation bundled) -> Installations {
+            bundled.version = ProbeServerVersion(bundled.executable);
+            const base::FilePath marker = execution_dir.AppendASCII(
+                browseros_server::kCurrentVersionFileName);
+            base::Version version = ReadReadyVersion(marker);
+            if (!version.IsValid()) {
+              base::DeleteFile(marker);
+              return {bundled, std::nullopt};
+            }
+            // A browser upgrade may supersede the downloaded copy. Do not run
+            // an obsolete binary just to discover a version we already know.
+            if (bundled.version.IsValid() && version <= bundled.version) {
+              base::DeleteFile(marker);
+              return {bundled, std::nullopt};
+            }
+            ServerInstallation ready =
+                DownloadedInstallation(execution_dir, binary_name, version);
+            const base::Version actual = ProbeServerVersion(ready.executable);
+            if (!actual.IsValid() || actual != version) {
+              LOG(WARNING) << "browseros: Rejecting invalid ready server "
+                           << version.GetString();
+              RemoveRejectedVersion(execution_dir, version);
+              return {bundled, std::nullopt};
+            }
+            return {bundled, ready};
+          },
+          execution_dir_, binary_name_, bundled_),
+      base::BindOnce(
+          [](base::WeakPtr<ServerVersionStore> store,
+             base::OnceClosure callback, Installations installations) {
+            if (!store) {
+              return;
+            }
+            store->bundled_ = std::move(installations.first);
+            store->ready_ = std::move(installations.second);
+            store->initialized_ = true;
+            std::move(callback).Run();
+          },
+          weak_factory_.GetWeakPtr(), std::move(callback)));
+}
+
+const ServerInstallation& ServerVersionStore::GetBestAvailable() const {
+  CHECK(initialized_);
+  if (ready_ &&
+      (!bundled_.version.IsValid() || ready_->version > bundled_.version)) {
+    return *ready_;
+  }
+  return bundled_;
+}
+
+void ServerVersionStore::PrepareVersion(
+    const base::Version& version,
+    base::OnceCallback<void(bool)> callback) {
+  CHECK(initialized_);
+  CHECK(version.IsValid());
+  ServerInstallation installation = GetDownloadedInstallation(version);
+  task_runner_->PostTaskAndReplyWithResult(
+      FROM_HERE,
+      base::BindOnce(
+          [](base::FilePath execution_dir,
+             ServerInstallation installation) -> bool {
+            const base::Version actual =
+                ProbeServerVersion(installation.executable);
+            if (!actual.IsValid() || actual != installation.version) {
+              RemoveRejectedVersion(execution_dir, installation.version);
+              return false;
+            }
+            // Serialize publication with initialization and rejection. An
+            // atomic marker write prevents partial versions across restarts.
+            return base::CreateDirectory(execution_dir) &&
+                   base::ImportantFileWriter::WriteFileAtomically(
+                       execution_dir.AppendASCII(
+                           browseros_server::kCurrentVersionFileName),
+                       installation.version.GetString());
+          },
+          execution_dir_, installation),
+      base::BindOnce(
+          [](base::WeakPtr<ServerVersionStore> store,
+             ServerInstallation installation,
+             base::OnceCallback<void(bool)> callback, bool success) {
+            if (!store) {
+              return;
+            }
+            if (success) {
+              store->ready_ = std::move(installation);
+            }
+            std::move(callback).Run(success);
+          },
+          weak_factory_.GetWeakPtr(), installation, std::move(callback)));
+}
+
+void ServerVersionStore::RejectVersion(const base::Version& version) {
+  if (!version.IsValid()) {
+    return;
+  }
+  if (ready_ && ready_->version == version) {
+    ready_.reset();
+  }
+  task_runner_->PostTask(FROM_HERE, base::BindOnce(&RemoveRejectedVersion,
+                                                   execution_dir_, version));
+}
+
+base::FilePath ServerVersionStore::GetVersionsDir() const {
+  return execution_dir_.AppendASCII(browseros_server::kVersionsDirectoryName);
+}
+
+base::FilePath ServerVersionStore::GetVersionDir(
+    const base::Version& version) const {
+  return GetVersionsDir().AppendASCII(version.GetString());
+}
+
+base::FilePath ServerVersionStore::GetPendingUpdateDir() const {
+  return execution_dir_.AppendASCII(
+      browseros_server::kPendingUpdateDirectoryName);
+}
+
+ServerInstallation ServerVersionStore::GetDownloadedInstallation(
+    const base::Version& version) const {
+  return DownloadedInstallation(execution_dir_, binary_name_, version);
+}
+
+}  // namespace browseros
