diff --git a/chrome/browser/browseros/extensions/browseros_extension_installer.cc b/chrome/browser/browseros/extensions/browseros_extension_installer.cc
new file mode 100644
index 0000000000000..56a3fb65d5348
--- /dev/null
+++ b/chrome/browser/browseros/extensions/browseros_extension_installer.cc
@@ -0,0 +1,298 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/extensions/browseros_extension_installer.h"
+
+#include <utility>
+
+#include "base/files/file_util.h"
+#include "base/json/json_reader.h"
+#include "base/logging.h"
+#include "base/path_service.h"
+#include "base/task/thread_pool.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
+#include "chrome/browser/extensions/external_provider_impl.h"
+#include "chrome/browser/profiles/profile.h"
+#include "chrome/common/chrome_paths.h"
+#include "content/public/browser/storage_partition.h"
+#include "net/base/load_flags.h"
+#include "net/traffic_annotation/network_traffic_annotation.h"
+#include "services/network/public/cpp/resource_request.h"
+#include "services/network/public/cpp/simple_url_loader.h"
+
+namespace browseros {
+
+namespace {
+
+constexpr net::NetworkTrafficAnnotationTag kTrafficAnnotation =
+    net::DefineNetworkTrafficAnnotation("browseros_extension_install", R"(
+        semantics {
+          sender: "BrowserOS Extension Installer"
+          description:
+            "Fetches JSON configuration specifying which extensions should "
+            "be installed for BrowserOS users."
+          trigger: "Browser startup when no bundled extensions available."
+          data: "No user data. GET request only."
+          destination: OTHER
+          destination_other: "BrowserOS configuration server."
+        }
+        policy {
+          cookies_allowed: NO
+          setting: "Controlled via command-line flags or enterprise policies."
+          policy_exception_justification: "BrowserOS feature."
+        })");
+
+}  // namespace
+
+InstallResult::InstallResult() = default;
+InstallResult::~InstallResult() = default;
+InstallResult::InstallResult(InstallResult&&) = default;
+InstallResult& InstallResult::operator=(InstallResult&&) = default;
+
+BrowserOSExtensionInstaller::BrowserOSExtensionInstaller(Profile* profile)
+    : profile_(profile) {
+  for (const std::string& id : GetBrowserOSExtensionIds()) {
+    extension_ids_.insert(id);
+  }
+}
+
+BrowserOSExtensionInstaller::~BrowserOSExtensionInstaller() = default;
+
+void BrowserOSExtensionInstaller::StartInstallation(
+    const GURL& config_url,
+    InstallCompleteCallback callback) {
+  config_url_ = config_url;
+  callback_ = std::move(callback);
+
+  LOG(INFO) << "browseros: Starting extension installation";
+
+  // TODO(nikhil): Re-enable bundled extension loading once OTA update flow is
+  // fully validated. Remote install is now fast with InstallPendingNow fix.
+#if 0
+  if (TryLoadFromBundled()) {
+    return;
+  }
+#endif
+
+  FetchFromRemote();
+}
+
+bool BrowserOSExtensionInstaller::TryLoadFromBundled() {
+  base::FilePath bundled_path;
+  if (!base::PathService::Get(chrome::DIR_BROWSEROS_BUNDLED_EXTENSIONS,
+                              &bundled_path)) {
+    LOG(INFO) << "browseros: Bundled path not available";
+    return false;
+  }
+
+  base::FilePath manifest_path =
+      bundled_path.Append(FILE_PATH_LITERAL("bundled_extensions.json"));
+
+  if (!base::PathExists(manifest_path)) {
+    LOG(INFO) << "browseros: No bundled manifest at " << manifest_path.value();
+    return false;
+  }
+
+  LOG(INFO) << "browseros: Loading from bundled at " << bundled_path.value();
+
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_BLOCKING},
+      base::BindOnce(&BrowserOSExtensionInstaller::ReadBundledManifest,
+                     manifest_path, bundled_path),
+      base::BindOnce(&BrowserOSExtensionInstaller::OnBundledLoadComplete,
+                     weak_ptr_factory_.GetWeakPtr(), bundled_path));
+
+  return true;
+}
+
+// static
+base::Value::Dict BrowserOSExtensionInstaller::ReadBundledManifest(
+    const base::FilePath& manifest_path,
+    const base::FilePath& bundled_path) {
+  std::string json_content;
+  if (!base::ReadFileToString(manifest_path, &json_content)) {
+    LOG(ERROR) << "browseros: Failed to read bundled manifest";
+    return base::Value::Dict();
+  }
+
+  std::optional<base::Value> parsed = base::JSONReader::Read(json_content);
+  if (!parsed || !parsed->is_dict()) {
+    LOG(ERROR) << "browseros: Invalid bundled manifest JSON";
+    return base::Value::Dict();
+  }
+
+  base::Value::Dict prefs;
+
+  for (const auto [extension_id, config] : parsed->GetDict()) {
+    if (!config.is_dict()) {
+      continue;
+    }
+
+    const base::Value::Dict& config_dict = config.GetDict();
+    const std::string* crx_file = config_dict.FindString("external_crx");
+    const std::string* version = config_dict.FindString("external_version");
+
+    if (!crx_file || !version) {
+      LOG(WARNING) << "browseros: Bundled config missing crx/version for "
+                   << extension_id;
+      continue;
+    }
+
+    base::FilePath crx_path =
+        bundled_path.Append(base::FilePath::FromUTF8Unsafe(*crx_file));
+
+    if (!base::PathExists(crx_path)) {
+      LOG(WARNING) << "browseros: CRX not found: " << crx_path.value();
+      continue;
+    }
+
+    base::Value::Dict ext_prefs;
+    ext_prefs.Set(extensions::ExternalProviderImpl::kExternalCrx,
+                  crx_path.AsUTF8Unsafe());
+    ext_prefs.Set(extensions::ExternalProviderImpl::kExternalVersion, *version);
+
+    prefs.Set(extension_id, std::move(ext_prefs));
+    LOG(INFO) << "browseros: Prepared bundled " << extension_id << " v"
+              << *version;
+  }
+
+  return prefs;
+}
+
+void BrowserOSExtensionInstaller::OnBundledLoadComplete(
+    const base::FilePath& bundled_path,
+    base::Value::Dict prefs) {
+  if (prefs.empty()) {
+    LOG(INFO) << "browseros: No valid bundled extensions, fetching remote";
+    FetchFromRemote();
+    return;
+  }
+
+  InstallResult result;
+  result.bundled_path = bundled_path;
+  result.from_bundled = true;
+  result.prefs = std::move(prefs);
+
+  for (const auto [extension_id, _] : result.prefs) {
+    result.extension_ids.insert(extension_id);
+  }
+
+  LOG(INFO) << "browseros: Loaded " << result.prefs.size()
+            << " bundled extensions";
+
+  Complete(std::move(result));
+}
+
+void BrowserOSExtensionInstaller::FetchFromRemote() {
+  if (!config_url_.is_valid()) {
+    LOG(ERROR) << "browseros: Invalid config URL";
+    Complete(InstallResult());
+    return;
+  }
+
+  LOG(INFO) << "browseros: Fetching config from " << config_url_.spec();
+
+  if (!url_loader_factory_) {
+    url_loader_factory_ = profile_->GetDefaultStoragePartition()
+                              ->GetURLLoaderFactoryForBrowserProcess();
+  }
+
+  auto request = std::make_unique<network::ResourceRequest>();
+  request->url = config_url_;
+  request->method = "GET";
+  request->load_flags = net::LOAD_BYPASS_CACHE | net::LOAD_DISABLE_CACHE;
+
+  url_loader_ =
+      network::SimpleURLLoader::Create(std::move(request), kTrafficAnnotation);
+
+  url_loader_->DownloadToStringOfUnboundedSizeUntilCrashAndDie(
+      url_loader_factory_.get(),
+      base::BindOnce(&BrowserOSExtensionInstaller::OnRemoteFetchComplete,
+                     weak_ptr_factory_.GetWeakPtr()));
+}
+
+void BrowserOSExtensionInstaller::OnRemoteFetchComplete(
+    std::unique_ptr<std::string> response_body) {
+  if (!response_body) {
+    LOG(ERROR) << "browseros: Failed to fetch config";
+    Complete(InstallResult());
+    return;
+  }
+
+  base::Value::Dict extensions_config = ParseConfigJson(*response_body);
+
+  if (extensions_config.empty()) {
+    Complete(InstallResult());
+    return;
+  }
+
+  InstallResult result;
+  result.config = extensions_config.Clone();
+  result.from_bundled = false;
+
+  for (const auto [extension_id, config] : extensions_config) {
+    if (!config.is_dict()) {
+      continue;
+    }
+
+    result.extension_ids.insert(extension_id);
+
+    const base::Value::Dict& config_dict = config.GetDict();
+    base::Value::Dict ext_prefs;
+
+    if (const std::string* update_url = config_dict.FindString(
+            extensions::ExternalProviderImpl::kExternalUpdateUrl)) {
+      ext_prefs.Set(extensions::ExternalProviderImpl::kExternalUpdateUrl,
+                    *update_url);
+    }
+
+    if (const std::string* crx = config_dict.FindString(
+            extensions::ExternalProviderImpl::kExternalCrx)) {
+      ext_prefs.Set(extensions::ExternalProviderImpl::kExternalCrx, *crx);
+    }
+
+    if (const std::string* version = config_dict.FindString(
+            extensions::ExternalProviderImpl::kExternalVersion)) {
+      ext_prefs.Set(extensions::ExternalProviderImpl::kExternalVersion,
+                    *version);
+    }
+
+    if (!ext_prefs.empty()) {
+      result.prefs.Set(extension_id, std::move(ext_prefs));
+    }
+  }
+
+  LOG(INFO) << "browseros: Loaded " << result.prefs.size()
+            << " extensions from remote config";
+
+  Complete(std::move(result));
+}
+
+base::Value::Dict BrowserOSExtensionInstaller::ParseConfigJson(
+    const std::string& json_content) {
+  std::optional<base::Value> parsed = base::JSONReader::Read(json_content);
+
+  if (!parsed || !parsed->is_dict()) {
+    LOG(ERROR) << "browseros: Invalid config JSON";
+    return base::Value::Dict();
+  }
+
+  const base::Value::Dict* extensions =
+      parsed->GetDict().FindDict("extensions");
+
+  if (!extensions) {
+    LOG(ERROR) << "browseros: No 'extensions' key in config";
+    return base::Value::Dict();
+  }
+
+  return extensions->Clone();
+}
+
+void BrowserOSExtensionInstaller::Complete(InstallResult result) {
+  if (callback_) {
+    std::move(callback_).Run(std::move(result));
+  }
+}
+
+}  // namespace browseros
