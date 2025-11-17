diff --git a/chrome/browser/extensions/blockbrowser_external_loader.cc b/chrome/browser/extensions/blockbrowser_external_loader.cc
new file mode 100644
index 0000000000000..aa11db933c978
--- /dev/null
+++ b/chrome/browser/extensions/blockbrowser_external_loader.cc
@@ -0,0 +1,645 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/extensions/blockbrowser_external_loader.h"
+
+#include <memory>
+#include <utility>
+
+#include "base/files/file_util.h"
+#include "base/functional/bind.h"
+#include "base/json/json_reader.h"
+#include "base/logging.h"
+#include "base/memory/ptr_util.h"
+#include "base/strings/string_util.h"
+#include "base/task/thread_pool.h"
+#include "base/task/single_thread_task_runner.h"
+#include "base/values.h"
+#include "chrome/browser/browser_process.h"
+#include "chrome/browser/extensions/blockbrowser_extension_constants.h"
+#include "chrome/browser/extensions/extension_service.h"
+#include "chrome/browser/extensions/external_provider_impl.h"
+#include "chrome/browser/extensions/updater/extension_updater.h"
+#include "chrome/browser/profiles/profile.h"
+#include "components/metrics/blockbrowser_metrics/blockbrowser_metrics.h"
+#include "content/public/browser/browser_context.h"
+#include "content/public/browser/storage_partition.h"
+#include "extensions/browser/disable_reason.h"
+#include "extensions/browser/extension_prefs.h"
+#include "extensions/browser/extension_registry.h"
+#include "extensions/browser/extension_system.h"
+#include "extensions/browser/pending_extension_manager.h"
+#include "extensions/common/extension.h"
+#include "extensions/common/mojom/manifest.mojom-shared.h"
+#include "net/base/load_flags.h"
+#include "net/traffic_annotation/network_traffic_annotation.h"
+#include "services/network/public/cpp/resource_request.h"
+#include "services/network/public/cpp/simple_url_loader.h"
+#include "services/network/public/mojom/url_response_head.mojom.h"
+
+namespace extensions {
+
+namespace {
+
+// Interval for periodic maintenance
+constexpr base::TimeDelta kPeriodicMaintenanceInterval = base::Minutes(15);
+
+// Network traffic annotation for the extension configuration fetch.
+constexpr net::NetworkTrafficAnnotationTag kBlockBrowserExtensionsFetchTrafficAnnotation =
+    net::DefineNetworkTrafficAnnotation("blockbrowser_extensions_fetch", R"(
+        semantics {
+          sender: "BlockBrowser External Extension Loader"
+          description:
+            "Fetches a JSON configuration file that specifies which extensions "
+            "should be installed for BlockBrowser users at startup."
+          trigger:
+            "Triggered during browser startup when BlockBrowser mode is enabled."
+          data:
+            "No user data is sent. Only a GET request to fetch the configuration."
+          destination: OTHER
+          destination_other:
+            "The BlockBrowser configuration server specified by the config URL."
+        }
+        policy {
+          cookies_allowed: NO
+          setting:
+            "This feature can be controlled via command-line flags or "
+            "enterprise policies."
+          policy_exception_justification:
+            "Not implemented yet. This is a new feature for BlockBrowser."
+        })");
+
+// Example JSON format:
+// {
+//   "extensions": {
+//     "extension_id_1": {
+//       "external_update_url": "https://example.com/extension1/updates.xml"
+//     },
+//     "extension_id_2": {
+//       "external_crx": "https://example.com/extension2.crx",
+//       "external_version": "1.0"
+//     }
+//   }
+// }
+
+}  // namespace
+
+BlockBrowserExternalLoader::BlockBrowserExternalLoader(Profile* profile)
+    : profile_(profile) {
+  // Default config URL - can be overridden via SetConfigUrl
+  config_url_ = GURL(browseros::kBlockBrowserConfigUrl);
+  
+  // Add known BlockBrowser extension IDs
+  for (const char* extension_id : browseros::kAllowedExtensions) {
+    blockbrowser_extension_ids_.insert(extension_id);
+  }
+}
+
+BlockBrowserExternalLoader::~BlockBrowserExternalLoader() = default;
+
+void BlockBrowserExternalLoader::StartLoading() {
+  LOG(INFO) << "BlockBrowser external extension loader starting...";
+
+  // Start periodic maintenance immediately, regardless of config state.
+  StartPeriodicCheck();
+  
+  if (!config_file_for_testing_.empty()) {
+    LoadFromFile();
+    return;
+  }
+
+  if (!config_url_.is_valid()) {
+    LOG(ERROR) << "Invalid BlockBrowser extensions config URL";
+    LoadFinished(base::Value::Dict());
+    return;
+  }
+  
+  LOG(INFO) << "Fetching BlockBrowser extensions from: " << config_url_.spec();
+
+  // Create the URL loader factory
+  url_loader_factory_ = profile_->GetDefaultStoragePartition()
+                            ->GetURLLoaderFactoryForBrowserProcess();
+
+  // Create the resource request
+  auto resource_request = std::make_unique<network::ResourceRequest>();
+  resource_request->url = config_url_;
+  resource_request->method = "GET";
+  resource_request->load_flags = net::LOAD_BYPASS_CACHE | net::LOAD_DISABLE_CACHE;
+
+  // Create the URL loader
+  url_loader_ = network::SimpleURLLoader::Create(
+      std::move(resource_request), kBlockBrowserExtensionsFetchTrafficAnnotation);
+
+  // Start the download
+  url_loader_->DownloadToStringOfUnboundedSizeUntilCrashAndDie(
+      url_loader_factory_.get(),
+      base::BindOnce(&BlockBrowserExternalLoader::OnURLFetchComplete,
+                     weak_ptr_factory_.GetWeakPtr()));
+}
+
+void BlockBrowserExternalLoader::OnURLFetchComplete(
+    std::unique_ptr<std::string> response_body) {
+  if (!response_body) {
+    LOG(ERROR) << "browseros: Failed to fetch BlockBrowser extensions config from "
+               << config_url_.spec();
+    if (has_successful_config_) {
+      LOG(WARNING) << "browseros: Keeping previously applied configuration.";
+    }
+    return;
+  }
+
+  if (!ParseConfiguration(*response_body)) {
+    LOG(ERROR) << "browseros: Initial configuration parse failed";
+  }
+}
+
+bool BlockBrowserExternalLoader::ParseConfiguration(
+    const std::string& json_content) {
+  std::optional<base::Value> parsed_json = base::JSONReader::Read(json_content);
+  if (!parsed_json || !parsed_json->is_dict()) {
+    LOG(ERROR) << "browseros: Failed to parse extensions config JSON";
+    return false;
+  }
+
+  const base::Value::Dict* extensions_dict =
+      parsed_json->GetDict().FindDict("extensions");
+  if (!extensions_dict) {
+    LOG(ERROR) << "browseros: No 'extensions' key found in BlockBrowser config";
+    return false;
+  }
+
+  base::Value::Dict filtered_config;
+  base::Value::Dict prefs;
+  size_t dropped_entries = 0u;
+
+  for (const auto [extension_id, extension_config] : *extensions_dict) {
+    if (!browseros::IsBlockBrowserExtension(extension_id)) {
+      ++dropped_entries;
+      continue;
+    }
+
+    if (!extension_config.is_dict()) {
+      LOG(WARNING) << "browseros: Invalid config for extension "
+                   << extension_id;
+      continue;
+    }
+
+    const base::Value::Dict& config_dict = extension_config.GetDict();
+
+    base::Value::Dict filtered_entry;
+    if (const std::string* update_url =
+            config_dict.FindString(ExternalProviderImpl::kExternalUpdateUrl)) {
+      filtered_entry.Set(ExternalProviderImpl::kExternalUpdateUrl, *update_url);
+    }
+    if (const std::string* crx_path =
+            config_dict.FindString(ExternalProviderImpl::kExternalCrx)) {
+      filtered_entry.Set(ExternalProviderImpl::kExternalCrx, *crx_path);
+    }
+    if (const std::string* version =
+            config_dict.FindString(ExternalProviderImpl::kExternalVersion)) {
+      filtered_entry.Set(ExternalProviderImpl::kExternalVersion, *version);
+    }
+    if (std::optional<bool> keep_if_present =
+            config_dict.FindBool(ExternalProviderImpl::kKeepIfPresent)) {
+      filtered_entry.Set(ExternalProviderImpl::kKeepIfPresent,
+                         keep_if_present.value());
+    }
+
+    // If update URL is missing, add it
+    if (!filtered_entry.contains(ExternalProviderImpl::kExternalUpdateUrl)) {
+      filtered_entry.Set(ExternalProviderImpl::kExternalUpdateUrl,
+                         browseros::kBlockBrowserUpdateUrl);
+    }
+
+    filtered_config.Set(extension_id, filtered_entry.Clone());
+    prefs.Set(extension_id, filtered_entry.Clone());
+  }
+
+  if (dropped_entries > 0) {
+    LOG(WARNING) << "browseros: Ignored " << dropped_entries
+                 << " non-allowlisted extensions in config";
+  }
+
+  if (prefs.empty()) {
+    LOG(ERROR) << "browseros: Allowlist produced no installable extensions";
+    return false;
+  }
+
+  if (filtered_config == last_config_) {
+    LOG(INFO) << "browseros: Config unchanged";
+    return true;
+  }
+
+  last_config_ = filtered_config.Clone();
+
+  blockbrowser_extension_ids_.clear();
+  for (const auto [extension_id, _] : filtered_config) {
+    blockbrowser_extension_ids_.insert(extension_id);
+  }
+
+  LOG(INFO) << "Loaded " << prefs.size() << " extensions from BlockBrowser config";
+
+  const bool had_previous_config = has_successful_config_;
+
+  LoadFinished(std::move(prefs));
+  has_successful_config_ = true;
+
+  if (!blockbrowser_extension_ids_.empty()) {
+    LOG(INFO) << "browseros: Triggering immediate high-priority installation for "
+              << blockbrowser_extension_ids_.size() << " BlockBrowser extensions";
+
+    base::SingleThreadTaskRunner::GetCurrentDefault()->PostDelayedTask(
+        FROM_HERE,
+        base::BindOnce(&BlockBrowserExternalLoader::TriggerImmediateInstallation,
+                       weak_ptr_factory_.GetWeakPtr()),
+        base::Seconds(2));
+  }
+
+  StartPeriodicCheck();
+  CheckAndLogExtensionState(had_previous_config ? "config_update" : "startup");
+
+  return true;
+}
+
+void BlockBrowserExternalLoader::StartPeriodicCheck() {
+  if (periodic_timer_.IsRunning()) {
+    return;
+  }
+
+  LOG(INFO) << "browseros: Starting periodic maintenance (every "
+            << kPeriodicMaintenanceInterval.InMinutes() << " minutes)";
+
+  periodic_timer_.Start(
+      FROM_HERE, kPeriodicMaintenanceInterval,
+      base::BindRepeating(&BlockBrowserExternalLoader::PeriodicMaintenance,
+                          weak_ptr_factory_.GetWeakPtr()));
+}
+
+void BlockBrowserExternalLoader::PeriodicMaintenance() {
+  LOG(INFO) << "browseros: Running periodic maintenance";
+  
+  if (!profile_) {
+    // Schedule next check even if profile isn't ready
+    StartPeriodicCheck();
+    return;
+  }
+  
+  // 1. Check for and reinstall any uninstalled BlockBrowser extensions
+  ReinstallUninstalledExtensions();
+  
+  // 2. Re-enable any disabled BlockBrowser extensions
+  ReenableDisabledExtensions();
+  
+  // 3. Fetch latest config and check for changes
+  FetchAndCheckConfig();
+  
+  // 4. Force immediate update check for all BlockBrowser extensions
+  ForceUpdateCheck();
+
+  // 5. Log extension state after all maintenance attempts
+  CheckAndLogExtensionState("periodic_maintenance");
+}
+
+void BlockBrowserExternalLoader::ReinstallUninstalledExtensions() {
+  ExtensionService* service = ExtensionSystem::Get(profile_)->extension_service();
+  if (!service) {
+    return;
+  }
+  
+  ExtensionRegistry* registry = ExtensionRegistry::Get(profile_);
+  PendingExtensionManager* pending_manager = PendingExtensionManager::Get(profile_);
+  
+  if (!registry || !pending_manager || last_config_.empty()) {
+    return;
+  }
+  
+  for (const std::string& extension_id : blockbrowser_extension_ids_) {
+    // Check if extension exists (installed or disabled)
+    if (registry->GetInstalledExtension(extension_id)) {
+      continue;  // Extension is installed, skip to next
+    }
+
+    if (registry->disabled_extensions().Contains(extension_id)) {
+      continue;  // Disabled extensions are handled separately
+    }
+    
+    LOG(INFO) << "browseros: Extension " << extension_id 
+              << " was uninstalled, attempting to reinstall";
+    
+    // Find the extension's configuration
+    const base::Value::Dict* extension_config = last_config_.FindDict(extension_id);
+    if (!extension_config) {
+      LOG(WARNING) << "browseros: No config found for " << extension_id;
+      continue;
+    }
+    
+    // Get the update URL from config
+    const std::string* update_url = 
+        extension_config->FindString(ExternalProviderImpl::kExternalUpdateUrl);
+    if (!update_url) {
+      LOG(WARNING) << "browseros: No update URL found for " << extension_id;
+      continue;
+    }
+    
+    // Validate and add to pending extensions
+    GURL update_gurl(*update_url);
+    if (!update_gurl.is_valid()) {
+      LOG(WARNING) << "browseros: Invalid update URL for " << extension_id;
+      continue;
+    }
+    
+    // Add as pending extension for installation
+    pending_manager->AddFromExternalUpdateUrl(
+        extension_id,
+        std::string(),  // No install param
+        update_gurl,
+        mojom::ManifestLocation::kExternalComponent,
+        Extension::WAS_INSTALLED_BY_DEFAULT,
+        false);
+    
+    LOG(INFO) << "browseros: Added " << extension_id 
+              << " to pending extensions for reinstall";
+    
+    // Trigger immediate installation
+    ExtensionUpdater* updater = ExtensionUpdater::Get(profile_);
+    if (updater) {
+      ExtensionUpdater::CheckParams params;
+      params.ids = {extension_id};
+      params.install_immediately = true;
+      params.fetch_priority = DownloadFetchPriority::kForeground;
+      updater->CheckNow(std::move(params));
+      LOG(INFO) << "browseros: Triggered immediate install for " << extension_id;
+    }
+  }
+}
+
+void BlockBrowserExternalLoader::ReenableDisabledExtensions() {
+  ExtensionService* service = ExtensionSystem::Get(profile_)->extension_service();
+  if (!service) {
+    return;
+  }
+  
+  ExtensionRegistry* registry = ExtensionRegistry::Get(profile_);
+  ExtensionPrefs* prefs = ExtensionPrefs::Get(profile_);
+  
+  if (!registry || !prefs) {
+    return;
+  }
+  
+  for (const std::string& extension_id : blockbrowser_extension_ids_) {
+    // Check if extension is disabled
+    if (!registry->disabled_extensions().Contains(extension_id)) {
+      continue;  // Extension is not disabled, skip to next
+    }
+
+    // Re-enable BlockBrowser extensions regardless of disable reason
+    LOG(INFO) << "browseros: Re-enabling extension " << extension_id;
+    service->EnableExtension(extension_id);
+  }
+}
+
+void BlockBrowserExternalLoader::FetchAndCheckConfig() {
+  LOG(INFO) << "browseros: Fetching latest config to check for changes";
+  
+  if (config_file_for_testing_.empty() && config_url_.is_valid()) {
+    // Fetch from URL
+    if (!url_loader_factory_) {
+      url_loader_factory_ = profile_->GetDefaultStoragePartition()
+                                ->GetURLLoaderFactoryForBrowserProcess();
+    }
+    
+    auto resource_request = std::make_unique<network::ResourceRequest>();
+    resource_request->url = config_url_;
+    resource_request->method = "GET";
+    resource_request->load_flags = net::LOAD_BYPASS_CACHE | net::LOAD_DISABLE_CACHE;
+    
+    auto config_check_loader = network::SimpleURLLoader::Create(
+        std::move(resource_request), kBlockBrowserExtensionsFetchTrafficAnnotation);
+    
+    // Store the loader to keep it alive during the request
+    auto* loader_ptr = config_check_loader.get();
+    loader_ptr->DownloadToStringOfUnboundedSizeUntilCrashAndDie(
+        url_loader_factory_.get(),
+        base::BindOnce(&BlockBrowserExternalLoader::OnConfigCheckComplete,
+                       weak_ptr_factory_.GetWeakPtr(),
+                       std::move(config_check_loader)));
+  }
+}
+
+void BlockBrowserExternalLoader::OnConfigCheckComplete(
+    std::unique_ptr<network::SimpleURLLoader> loader,
+    std::unique_ptr<std::string> response_body) {
+  if (!response_body) {
+    LOG(WARNING) << "browseros: Failed to fetch config for update check";
+    return;
+  }
+
+  if (!ParseConfiguration(*response_body)) {
+    LOG(WARNING) << "browseros: Ignoring config update due to parse failure";
+  }
+}
+
+void BlockBrowserExternalLoader::TriggerImmediateInstallation() {
+  if (!profile_ || blockbrowser_extension_ids_.empty()) {
+    return;
+  }
+  
+  LOG(INFO) << "browseros: Triggering immediate installation on first start";
+  
+  // First, add all extensions to pending if they're not already installed
+  ExtensionRegistry* registry = ExtensionRegistry::Get(profile_);
+  PendingExtensionManager* pending_manager = PendingExtensionManager::Get(profile_);
+  
+  if (registry && pending_manager && !last_config_.empty()) {
+    for (const std::string& extension_id : blockbrowser_extension_ids_) {
+      // Skip if already installed
+      if (registry->GetInstalledExtension(extension_id)) {
+        LOG(INFO) << "browseros: Extension " << extension_id << " already installed";
+        continue;
+      }
+      
+      // Add to pending extensions
+      const base::Value::Dict* extension_config = last_config_.FindDict(extension_id);
+      if (extension_config) {
+        const std::string* update_url = 
+            extension_config->FindString(ExternalProviderImpl::kExternalUpdateUrl);
+        if (update_url) {
+          GURL update_gurl(*update_url);
+          if (update_gurl.is_valid()) {
+            pending_manager->AddFromExternalUpdateUrl(
+                extension_id,
+                std::string(),  // No install param
+                update_gurl,
+                mojom::ManifestLocation::kExternalComponent,
+                Extension::WAS_INSTALLED_BY_DEFAULT,
+                false);  // Don't mark acknowledged
+            LOG(INFO) << "browseros: Added " << extension_id 
+                      << " to pending for immediate installation";
+          }
+        }
+      }
+    }
+  }
+  
+  // Now trigger immediate high-priority installation
+  ExtensionUpdater* updater = ExtensionUpdater::Get(profile_);
+  if (!updater) {
+    LOG(WARNING) << "browseros: No extension updater available for immediate installation";
+    return;
+  }
+  
+  LOG(INFO) << "browseros: Executing CheckNow with immediate install for " 
+            << blockbrowser_extension_ids_.size() << " BlockBrowser extensions";
+  
+  // Create CheckParams for immediate foreground installation
+  ExtensionUpdater::CheckParams params;
+  params.ids = std::list<ExtensionId>(blockbrowser_extension_ids_.begin(), 
+                                       blockbrowser_extension_ids_.end());
+  params.install_immediately = true;
+  params.fetch_priority = DownloadFetchPriority::kForeground;
+  
+  // Trigger the installation
+  updater->CheckNow(std::move(params));
+}
+
+void BlockBrowserExternalLoader::ForceUpdateCheck() {
+  if (!profile_ || blockbrowser_extension_ids_.empty()) {
+    return;
+  }
+  
+  ExtensionUpdater* updater = ExtensionUpdater::Get(profile_);
+  if (!updater) {
+    LOG(WARNING) << "browseros: No extension updater available";
+    return;
+  }
+  
+  LOG(INFO) << "browseros: Forcing immediate update check for " 
+            << blockbrowser_extension_ids_.size() << " BlockBrowser extensions";
+  
+  // Create CheckParams for immediate foreground update
+  ExtensionUpdater::CheckParams params;
+  params.ids = std::list<ExtensionId>(blockbrowser_extension_ids_.begin(), 
+                                       blockbrowser_extension_ids_.end());
+  params.install_immediately = true;
+  params.fetch_priority = DownloadFetchPriority::kForeground;
+  
+  // Trigger the update check
+  updater->CheckNow(std::move(params));
+}
+
+void BlockBrowserExternalLoader::LoadFromFile() {
+  // This runs on a background thread to avoid blocking the UI
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE,
+      {base::MayBlock(), base::TaskPriority::USER_VISIBLE},
+      base::BindOnce([](const base::FilePath& path) -> std::string {
+        std::string contents;
+        if (!base::ReadFileToString(path, &contents)) {
+          LOG(ERROR) << "Failed to read BlockBrowser config file: " << path;
+          return std::string();
+        }
+        return contents;
+      },
+      config_file_for_testing_),
+      base::BindOnce(&BlockBrowserExternalLoader::OnConfigFileLoaded,
+                     weak_ptr_factory_.GetWeakPtr()));
+}
+
+void BlockBrowserExternalLoader::OnConfigFileLoaded(std::string contents) {
+  if (contents.empty()) {
+    LOG(ERROR) << "browseros: BlockBrowser config file is empty";
+    return;
+  }
+
+  if (!ParseConfiguration(contents)) {
+    LOG(ERROR) << "browseros: Failed to parse BlockBrowser config file";
+  }
+}
+
+void BlockBrowserExternalLoader::CheckAndLogExtensionState(
+    const std::string& context) {
+  if (!profile_) {
+    return;
+  }
+
+  ExtensionRegistry* registry = ExtensionRegistry::Get(profile_);
+  ExtensionPrefs* prefs = ExtensionPrefs::Get(profile_);
+
+  if (!registry || !prefs) {
+    return;
+  }
+
+  for (const std::string& extension_id : blockbrowser_extension_ids_) {
+    // If extension is enabled, it's healthy - skip logging
+    if (registry->enabled_extensions().Contains(extension_id)) {
+      continue;
+    }
+
+    // Extension is NOT enabled - gather diagnostic information
+    base::Value::Dict properties;
+    properties.Set("extension_id", extension_id);
+    properties.Set("context", context);
+
+    std::string state;
+
+    if (registry->disabled_extensions().Contains(extension_id)) {
+      state = "disabled";
+
+      // Get extension version if available
+      const Extension* extension = registry->disabled_extensions().GetByID(extension_id);
+      if (extension) {
+        properties.Set("version", extension->version().GetString());
+      }
+
+      // Get disable reasons using public API
+      DisableReasonSet disable_reasons = prefs->GetDisableReasons(extension_id);
+
+      // Convert to bitmask by ORing all reason values
+      int bitmask = 0;
+      for (disable_reason::DisableReason reason : disable_reasons) {
+        bitmask |= static_cast<int>(reason);
+      }
+      properties.Set("disable_reasons_bitmask", bitmask);
+
+      // Log individual disable reason flags for easy querying
+      properties.Set("reason_user_action",
+                     disable_reasons.contains(disable_reason::DISABLE_USER_ACTION));
+      properties.Set("reason_permissions_increase",
+                     disable_reasons.contains(disable_reason::DISABLE_PERMISSIONS_INCREASE));
+      properties.Set("reason_reload",
+                     disable_reasons.contains(disable_reason::DISABLE_RELOAD));
+      properties.Set("reason_corrupted",
+                     disable_reasons.contains(disable_reason::DISABLE_CORRUPTED));
+      properties.Set("reason_greylist",
+                     disable_reasons.contains(disable_reason::DISABLE_GREYLIST));
+      properties.Set("reason_remote_install",
+                     disable_reasons.contains(disable_reason::DISABLE_REMOTE_INSTALL));
+
+    } else if (registry->blocklisted_extensions().Contains(extension_id)) {
+      state = "blocklisted";
+
+    } else if (registry->blocked_extensions().Contains(extension_id)) {
+      state = "blocked";
+
+    } else if (registry->terminated_extensions().Contains(extension_id)) {
+      state = "terminated";
+
+    } else {
+      state = "not_installed";
+    }
+
+    properties.Set("state", state);
+
+    // Log to metrics
+    blockbrowser_metrics::BlockBrowserMetrics::Log("ota.extension.unexpected_state",
+                                              std::move(properties));
+
+    // Also log to Chrome logs for local debugging
+    LOG(WARNING) << "browseros: Extension " << extension_id
+                 << " in unexpected state: " << state
+                 << " (context: " << context << ")";
+  }
+}
+
+}  // namespace extensions
