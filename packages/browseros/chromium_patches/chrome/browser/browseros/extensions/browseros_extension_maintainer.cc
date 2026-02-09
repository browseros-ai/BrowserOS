diff --git a/chrome/browser/browseros/extensions/browseros_extension_maintainer.cc b/chrome/browser/browseros/extensions/browseros_extension_maintainer.cc
new file mode 100644
index 0000000000000..404fb31de4e55
--- /dev/null
+++ b/chrome/browser/browseros/extensions/browseros_extension_maintainer.cc
@@ -0,0 +1,383 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/extensions/browseros_extension_maintainer.h"
+
+#include <optional>
+#include <utility>
+
+#include "base/json/json_reader.h"
+#include "base/logging.h"
+#include "base/task/single_thread_task_runner.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics.h"
+#include "chrome/browser/extensions/extension_service.h"
+#include "chrome/browser/extensions/external_provider_impl.h"
+#include "chrome/browser/extensions/updater/extension_updater.h"
+#include "chrome/browser/profiles/profile.h"
+#include "content/public/browser/storage_partition.h"
+#include "extensions/browser/disable_reason.h"
+#include "extensions/browser/extension_prefs.h"
+#include "extensions/browser/extension_registrar.h"
+#include "extensions/browser/extension_registry.h"
+#include "extensions/browser/pending_extension_manager.h"
+#include "extensions/browser/uninstall_reason.h"
+#include "extensions/common/extension.h"
+#include "extensions/common/mojom/manifest.mojom-shared.h"
+#include "net/base/load_flags.h"
+#include "net/traffic_annotation/network_traffic_annotation.h"
+#include "services/network/public/cpp/resource_request.h"
+#include "services/network/public/cpp/simple_url_loader.h"
+
+namespace browseros {
+
+namespace {
+
+constexpr base::TimeDelta kMaintenanceInterval = base::Minutes(15);
+constexpr base::TimeDelta kInitialMaintenanceDelay = base::Seconds(60);
+
+constexpr net::NetworkTrafficAnnotationTag kTrafficAnnotation =
+    net::DefineNetworkTrafficAnnotation("browseros_extension_maintenance", R"(
+        semantics {
+          sender: "BrowserOS Extension Maintainer"
+          description:
+            "Fetches JSON configuration for BrowserOS extension maintenance."
+          trigger: "Periodic maintenance cycle (every 15 minutes)."
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
+BrowserOSExtensionMaintainer::BrowserOSExtensionMaintainer(Profile* profile)
+    : profile_(profile) {}
+
+BrowserOSExtensionMaintainer::~BrowserOSExtensionMaintainer() = default;
+
+void BrowserOSExtensionMaintainer::Start(const GURL& config_url,
+                                         std::set<std::string> extension_ids,
+                                         base::Value::Dict initial_config) {
+  config_url_ = config_url;
+  extension_ids_ = std::move(extension_ids);
+  last_config_ = std::move(initial_config);
+
+  LOG(INFO) << "browseros: Scheduling maintenance in "
+            << kInitialMaintenanceDelay.InSeconds() << " seconds";
+
+  base::SingleThreadTaskRunner::GetCurrentDefault()->PostDelayedTask(
+      FROM_HERE,
+      base::BindOnce(&BrowserOSExtensionMaintainer::RunMaintenanceCycle,
+                     weak_ptr_factory_.GetWeakPtr()),
+      kInitialMaintenanceDelay);
+}
+
+void BrowserOSExtensionMaintainer::UpdateExtensionIds(
+    std::set<std::string> ids) {
+  extension_ids_ = std::move(ids);
+}
+
+void BrowserOSExtensionMaintainer::RunMaintenanceCycle() {
+  LOG(INFO) << "browseros: Running maintenance cycle";
+
+  if (!profile_) {
+    ScheduleNextMaintenance();
+    return;
+  }
+
+  if (!config_url_.is_valid()) {
+    ExecuteMaintenanceTasks();
+    ScheduleNextMaintenance();
+    return;
+  }
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
+  auto loader =
+      network::SimpleURLLoader::Create(std::move(request), kTrafficAnnotation);
+
+  auto* loader_ptr = loader.get();
+  loader_ptr->DownloadToStringOfUnboundedSizeUntilCrashAndDie(
+      url_loader_factory_.get(),
+      base::BindOnce(&BrowserOSExtensionMaintainer::OnConfigFetched,
+                     weak_ptr_factory_.GetWeakPtr(), std::move(loader)));
+}
+
+void BrowserOSExtensionMaintainer::OnConfigFetched(
+    std::unique_ptr<network::SimpleURLLoader> loader,
+    std::optional<std::string> response_body) {
+  if (response_body.has_value()) {
+    base::Value::Dict config = ParseConfigJson(*response_body);
+    if (!config.empty()) {
+      last_config_ = std::move(config);
+
+      for (const auto [id, _] : last_config_) {
+        extension_ids_.insert(id);
+      }
+
+      LOG(INFO) << "browseros: Updated config with " << last_config_.size()
+                << " extensions";
+    }
+  } else {
+    LOG(WARNING) << "browseros: Failed to fetch maintenance config";
+  }
+
+  ExecuteMaintenanceTasks();
+  ScheduleNextMaintenance();
+}
+
+base::Value::Dict BrowserOSExtensionMaintainer::ParseConfigJson(
+    const std::string& json_content) {
+  std::optional<base::Value> parsed =
+      base::JSONReader::Read(json_content, base::JSON_PARSE_RFC);
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
+void BrowserOSExtensionMaintainer::ExecuteMaintenanceTasks() {
+  LOG(INFO) << "browseros: Executing maintenance tasks";
+
+  UninstallDeprecatedExtensions();
+  ReinstallMissingExtensions();
+  ReenableDisabledExtensions();
+  ForceUpdateCheck();
+  LogExtensionHealth("maintenance");
+}
+
+void BrowserOSExtensionMaintainer::ScheduleNextMaintenance() {
+  LOG(INFO) << "browseros: Scheduling next maintenance in "
+            << kMaintenanceInterval.InMinutes() << " minutes";
+
+  base::SingleThreadTaskRunner::GetCurrentDefault()->PostDelayedTask(
+      FROM_HERE,
+      base::BindOnce(&BrowserOSExtensionMaintainer::RunMaintenanceCycle,
+                     weak_ptr_factory_.GetWeakPtr()),
+      kMaintenanceInterval);
+}
+
+void BrowserOSExtensionMaintainer::UninstallDeprecatedExtensions() {
+  if (!profile_ || last_config_.empty()) {
+    return;
+  }
+
+  extensions::ExtensionRegistry* registry =
+      extensions::ExtensionRegistry::Get(profile_);
+  extensions::ExtensionRegistrar* registrar =
+      extensions::ExtensionRegistrar::Get(profile_);
+
+  if (!registry || !registrar) {
+    return;
+  }
+
+  std::set<std::string> server_ids;
+  for (const auto [id, _] : last_config_) {
+    server_ids.insert(id);
+  }
+
+  for (const std::string& id : GetBrowserOSExtensionIds()) {
+    if (server_ids.contains(id)) {
+      continue;
+    }
+
+    const extensions::Extension* ext = registry->GetInstalledExtension(id);
+    if (!ext) {
+      continue;
+    }
+
+    LOG(INFO) << "browseros: Uninstalling deprecated extension " << id;
+
+    std::u16string error;
+    if (!registrar->UninstallExtension(
+            id, extensions::UNINSTALL_REASON_ORPHANED_EXTERNAL_EXTENSION,
+            &error)) {
+      LOG(WARNING) << "browseros: Failed to uninstall " << id << ": " << error;
+    }
+  }
+}
+
+void BrowserOSExtensionMaintainer::ReinstallMissingExtensions() {
+  if (!profile_ || last_config_.empty()) {
+    return;
+  }
+
+  extensions::ExtensionRegistry* registry =
+      extensions::ExtensionRegistry::Get(profile_);
+  extensions::PendingExtensionManager* pending =
+      extensions::PendingExtensionManager::Get(profile_);
+
+  if (!registry || !pending) {
+    return;
+  }
+
+  for (const std::string& id : extension_ids_) {
+    if (registry->GetInstalledExtension(id)) {
+      continue;
+    }
+
+    const base::Value::Dict* config = last_config_.FindDict(id);
+    if (!config) {
+      continue;
+    }
+
+    const std::string* update_url = config->FindString(
+        extensions::ExternalProviderImpl::kExternalUpdateUrl);
+    if (!update_url) {
+      continue;
+    }
+
+    GURL url(*update_url);
+    if (!url.is_valid()) {
+      continue;
+    }
+
+    LOG(INFO) << "browseros: Reinstalling missing extension " << id;
+
+    pending->AddFromExternalUpdateUrl(
+        id, std::string(), url,
+        extensions::mojom::ManifestLocation::kExternalComponent,
+        extensions::Extension::WAS_INSTALLED_BY_DEFAULT, false);
+
+    extensions::ExtensionUpdater* updater =
+        extensions::ExtensionUpdater::Get(profile_);
+    if (updater) {
+      extensions::ExtensionUpdater::CheckParams params;
+      params.ids = {id};
+      params.install_immediately = true;
+      params.fetch_priority =
+          extensions::DownloadFetchPriority::kForeground;
+      // Use InstallPendingNow - the extension is in PendingExtensionManager,
+      // CheckNow with specific IDs only checks installed extensions.
+      updater->InstallPendingNow(std::move(params));
+    }
+  }
+}
+
+void BrowserOSExtensionMaintainer::ReenableDisabledExtensions() {
+  if (!profile_) {
+    return;
+  }
+
+  extensions::ExtensionRegistry* registry =
+      extensions::ExtensionRegistry::Get(profile_);
+  extensions::ExtensionRegistrar* registrar =
+      extensions::ExtensionRegistrar::Get(profile_);
+
+  if (!registry || !registrar) {
+    return;
+  }
+
+  for (const std::string& id : extension_ids_) {
+    if (!registry->disabled_extensions().Contains(id)) {
+      continue;
+    }
+
+    LOG(INFO) << "browseros: Re-enabling disabled extension " << id;
+    registrar->EnableExtension(id);
+  }
+}
+
+void BrowserOSExtensionMaintainer::ForceUpdateCheck() {
+  if (!profile_ || extension_ids_.empty()) {
+    return;
+  }
+
+  extensions::ExtensionUpdater* updater =
+      extensions::ExtensionUpdater::Get(profile_);
+  if (!updater) {
+    return;
+  }
+
+  LOG(INFO) << "browseros: Forcing update check for " << extension_ids_.size()
+            << " extensions";
+
+  extensions::ExtensionUpdater::CheckParams params;
+  params.ids = std::list<extensions::ExtensionId>(extension_ids_.begin(),
+                                                   extension_ids_.end());
+  params.install_immediately = true;
+  params.fetch_priority = extensions::DownloadFetchPriority::kForeground;
+  updater->CheckNow(std::move(params));
+}
+
+void BrowserOSExtensionMaintainer::LogExtensionHealth(
+    const std::string& context) {
+  if (!profile_) {
+    return;
+  }
+
+  extensions::ExtensionRegistry* registry =
+      extensions::ExtensionRegistry::Get(profile_);
+  extensions::ExtensionPrefs* prefs =
+      extensions::ExtensionPrefs::Get(profile_);
+
+  if (!registry || !prefs) {
+    return;
+  }
+
+  for (const std::string& id : extension_ids_) {
+    if (registry->enabled_extensions().Contains(id)) {
+      continue;
+    }
+
+    std::string state;
+    base::Value::Dict properties;
+    properties.Set("extension_id", id);
+    properties.Set("context", context);
+
+    if (registry->disabled_extensions().Contains(id)) {
+      state = "disabled";
+
+      extensions::DisableReasonSet reasons = prefs->GetDisableReasons(id);
+      int bitmask = 0;
+      for (extensions::disable_reason::DisableReason reason : reasons) {
+        bitmask |= static_cast<int>(reason);
+      }
+      properties.Set("disable_reasons_bitmask", bitmask);
+
+    } else if (registry->blocklisted_extensions().Contains(id)) {
+      state = "blocklisted";
+    } else if (registry->blocked_extensions().Contains(id)) {
+      state = "blocked";
+    } else if (registry->terminated_extensions().Contains(id)) {
+      state = "terminated";
+    } else {
+      state = "not_installed";
+    }
+
+    properties.Set("state", state);
+
+    browseros_metrics::BrowserOSMetrics::Log("ota.extension.unexpected_state",
+                                             std::move(properties));
+
+    LOG(WARNING) << "browseros: Extension " << id << " in state: " << state
+                 << " (context: " << context << ")";
+  }
+}
+
+}  // namespace browseros
