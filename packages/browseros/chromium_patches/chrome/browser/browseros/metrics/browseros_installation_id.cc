diff --git a/chrome/browser/browseros/metrics/browseros_installation_id.cc b/chrome/browser/browseros/metrics/browseros_installation_id.cc
new file mode 100644
index 0000000000000000000000000000000000000000..2d9259faba70aba351344624fcb70d77af97c2cf
--- /dev/null
+++ b/chrome/browser/browseros/metrics/browseros_installation_id.cc
@@ -0,0 +1,129 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/metrics/browseros_installation_id.h"
+
+#include <optional>
+#include <string>
+
+#include "base/files/file.h"
+#include "base/files/file_util.h"
+#include "base/functional/bind.h"
+#include "base/functional/callback_helpers.h"
+#include "base/json/json_reader.h"
+#include "base/json/json_writer.h"
+#include "base/logging.h"
+#include "base/uuid.h"
+#include "base/values.h"
+#include "build/build_config.h"
+
+#if BUILDFLAG(IS_POSIX)
+#include <unistd.h>
+#endif
+
+namespace browseros_metrics {
+namespace {
+
+constexpr base::FilePath::StringViewType kInstallationFileName =
+    FILE_PATH_LITERAL("installation.json");
+constexpr char kInstallIdKey[] = "install_id";
+
+std::optional<std::string> ReadInstallationId(
+    const base::FilePath& installation_path) {
+  std::string contents;
+  if (!base::ReadFileToString(installation_path, &contents)) {
+    return std::nullopt;
+  }
+
+  std::optional<base::DictValue> installation =
+      base::JSONReader::ReadDict(contents, base::JSON_PARSE_RFC);
+  const std::string* install_id =
+      installation ? installation->FindString(kInstallIdKey) : nullptr;
+  if (!install_id ||
+      !base::Uuid::ParseCaseInsensitive(*install_id).is_valid()) {
+    LOG(ERROR) << "browseros: Invalid installation identity file: "
+               << installation_path;
+    return std::nullopt;
+  }
+  return *install_id;
+}
+
+bool PublishInstallationFile(const base::FilePath& temporary_path,
+                             const base::FilePath& installation_path) {
+#if BUILDFLAG(IS_WIN)
+  return base::CreateWinHardLink(installation_path, temporary_path);
+#elif BUILDFLAG(IS_POSIX)
+  return ::link(temporary_path.value().c_str(),
+                installation_path.value().c_str()) == 0;
+#else
+  return false;
+#endif
+}
+
+}  // namespace
+
+std::optional<std::string> LoadOrCreateInstallationId(
+    const base::FilePath& product_state_directory) {
+  if (product_state_directory.empty()) {
+    return std::nullopt;
+  }
+
+  const base::FilePath installation_path =
+      product_state_directory.Append(kInstallationFileName);
+  if (base::PathExists(installation_path)) {
+    return ReadInstallationId(installation_path);
+  }
+
+  if (!base::CreateDirectory(product_state_directory)) {
+    LOG(ERROR) << "browseros: Failed to create product state directory: "
+               << product_state_directory;
+    return std::nullopt;
+  }
+
+  const std::string candidate_id =
+      base::Uuid::GenerateRandomV4().AsLowercaseString();
+  base::DictValue installation;
+  installation.Set(kInstallIdKey, candidate_id);
+  std::optional<std::string> json = base::WriteJson(installation);
+  if (!json.has_value()) {
+    return std::nullopt;
+  }
+  json->push_back('\n');
+
+  base::FilePath temporary_path;
+  if (!base::CreateTemporaryFileInDir(product_state_directory,
+                                      &temporary_path)) {
+    LOG(ERROR) << "browseros: Failed to create temporary installation file";
+    return std::nullopt;
+  }
+  base::ScopedClosureRunner delete_temporary_file(base::BindOnce(
+      [](base::FilePath path) { base::DeleteFile(path); }, temporary_path));
+
+  if (!base::WriteFile(temporary_path, *json)) {
+    LOG(ERROR) << "browseros: Failed to write temporary installation file";
+    return std::nullopt;
+  }
+  base::File temporary_file(temporary_path,
+                            base::File::FLAG_OPEN | base::File::FLAG_WRITE);
+  if (!temporary_file.IsValid() || !temporary_file.Flush()) {
+    LOG(ERROR) << "browseros: Failed to flush temporary installation file";
+    return std::nullopt;
+  }
+  temporary_file.Close();
+
+  // A hard link publishes the complete temporary file only when the
+  // destination is absent. If another process won the race, adopt its ID.
+  if (PublishInstallationFile(temporary_path, installation_path)) {
+    return candidate_id;
+  }
+  if (base::PathExists(installation_path)) {
+    return ReadInstallationId(installation_path);
+  }
+
+  LOG(ERROR) << "browseros: Failed to publish installation identity: "
+             << installation_path;
+  return std::nullopt;
+}
+
+}  // namespace browseros_metrics
