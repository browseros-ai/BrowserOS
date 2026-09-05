diff --git a/chrome/browser/browseros/extensions/browseros_extension_installer.cc b/chrome/browser/browseros/extensions/browseros_extension_installer.cc
new file mode 100644
index 0000000000000000000000000000000000000000..2b60bbcfad7a34e6366be030d15bdb6adf81ed8b
--- /dev/null
+++ b/chrome/browser/browseros/extensions/browseros_extension_installer.cc
@@ -0,0 +1,131 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/extensions/browseros_extension_installer.h"
+
+#include <optional>
+#include <utility>
+
+#include "base/files/file_util.h"
+#include "base/functional/bind.h"
+#include "base/json/json_reader.h"
+#include "base/logging.h"
+#include "base/path_service.h"
+#include "base/task/thread_pool.h"
+#include "base/version.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
+#include "chrome/browser/extensions/external_provider_impl.h"
+#include "chrome/common/chrome_paths.h"
+#include "content/public/browser/browser_thread.h"
+
+namespace browseros {
+
+namespace {
+
+constexpr size_t kMaxBundledManifestBytes = 1024 * 1024;
+
+}  // namespace
+
+BundleDiscoveryResult::BundleDiscoveryResult() = default;
+BundleDiscoveryResult::~BundleDiscoveryResult() = default;
+BundleDiscoveryResult::BundleDiscoveryResult(BundleDiscoveryResult&&) = default;
+BundleDiscoveryResult& BundleDiscoveryResult::operator=(
+    BundleDiscoveryResult&&) = default;
+
+BrowserOSExtensionInstaller::BrowserOSExtensionInstaller() = default;
+BrowserOSExtensionInstaller::~BrowserOSExtensionInstaller() = default;
+
+void BrowserOSExtensionInstaller::DiscoverBundled(DiscoveryCallback callback) {
+  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+
+  // Resolve product membership on UI, then perform all filesystem work on the
+  // pool. USER_VISIBLE keeps first-run discovery prompt without blocking input.
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_VISIBLE},
+      base::BindOnce(&BrowserOSExtensionInstaller::ReadBundledManifest,
+                     GetActiveBrowserOSExtensionIds()),
+      base::BindOnce(&BrowserOSExtensionInstaller::OnBundledLoadComplete,
+                     weak_ptr_factory_.GetWeakPtr(), std::move(callback)));
+}
+
+// static
+BundleDiscoveryResult BrowserOSExtensionInstaller::ReadBundledManifest(
+    std::vector<std::string> active_ids) {
+  BundleDiscoveryResult result;
+  if (!base::PathService::Get(chrome::DIR_BROWSEROS_BUNDLED_EXTENSIONS,
+                              &result.bundled_path)) {
+    LOG(WARNING) << "browseros: Bundled extension path unavailable";
+    return result;
+  }
+
+  const base::FilePath manifest_path =
+      result.bundled_path.Append(FILE_PATH_LITERAL("bundled_extensions.json"));
+  std::string json_content;
+  if (!base::ReadFileToStringWithMaxSize(manifest_path, &json_content,
+                                         kMaxBundledManifestBytes)) {
+    LOG(WARNING) << "browseros: Bundled manifest missing or unreadable at "
+                 << manifest_path.value();
+    return result;
+  }
+
+  std::optional<base::Value> parsed =
+      base::JSONReader::Read(json_content, base::JSON_PARSE_RFC);
+  if (!parsed || !parsed->is_dict()) {
+    LOG(ERROR) << "browseros: Invalid bundled manifest JSON";
+    return result;
+  }
+
+  // Iterate the active catalog rather than trusting manifest membership. A
+  // valid secondary entry must not make a missing primary look like success.
+  for (const std::string& id : active_ids) {
+    const base::DictValue* config = parsed->GetDict().FindDict(id);
+    if (!config) {
+      continue;
+    }
+    const std::string* crx_file = config->FindString("external_crx");
+    const std::string* version = config->FindString("external_version");
+    if (!crx_file || !version || !base::Version(*version).IsValid()) {
+      LOG(WARNING) << "browseros: Invalid bundled crx/version for " << id;
+      continue;
+    }
+
+    const base::FilePath relative_path =
+        base::FilePath::FromUTF8Unsafe(*crx_file);
+    if (relative_path.empty() || relative_path.IsAbsolute() ||
+        relative_path.ReferencesParent() ||
+        !relative_path.MatchesExtension(FILE_PATH_LITERAL(".crx"))) {
+      LOG(WARNING) << "browseros: Invalid bundled CRX path for " << id;
+      continue;
+    }
+
+    const base::FilePath crx_path = result.bundled_path.Append(relative_path);
+    base::File::Info info;
+    if (!base::GetFileInfo(crx_path, &info) || info.is_directory ||
+        info.size <= 0 || base::IsLink(crx_path)) {
+      LOG(WARNING) << "browseros: Bundled CRX missing or invalid for " << id;
+      continue;
+    }
+
+    base::DictValue prefs;
+    prefs.Set(extensions::ExternalProviderImpl::kExternalCrx,
+              crx_path.AsUTF8Unsafe());
+    prefs.Set(extensions::ExternalProviderImpl::kExternalVersion, *version);
+    result.prefs.Set(id, std::move(prefs));
+  }
+
+  result.complete =
+      !active_ids.empty() && result.prefs.size() == active_ids.size();
+  return result;
+}
+
+void BrowserOSExtensionInstaller::OnBundledLoadComplete(
+    DiscoveryCallback callback,
+    BundleDiscoveryResult result) {
+  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+  LOG(INFO) << "browseros: Local extension metadata ready, "
+            << result.prefs.size() << " entries; complete=" << result.complete;
+  std::move(callback).Run(std::move(result));
+}
+
+}  // namespace browseros
