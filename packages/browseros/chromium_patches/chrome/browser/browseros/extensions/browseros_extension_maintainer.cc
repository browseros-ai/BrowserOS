diff --git a/chrome/browser/browseros/extensions/browseros_extension_maintainer.cc b/chrome/browser/browseros/extensions/browseros_extension_maintainer.cc
new file mode 100644
index 0000000000000000000000000000000000000000..f061377c17fccfdaf687ab5201b4c82f6dea3fcc
--- /dev/null
+++ b/chrome/browser/browseros/extensions/browseros_extension_maintainer.cc
@@ -0,0 +1,292 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/extensions/browseros_extension_maintainer.h"
+
+#include <optional>
+#include <utility>
+
+#include "base/functional/bind.h"
+#include "base/json/json_reader.h"
+#include "base/logging.h"
+#include "base/time/time.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics.h"
+#include "chrome/browser/extensions/external_provider_impl.h"
+#include "chrome/browser/profiles/profile.h"
+#include "content/public/browser/browser_thread.h"
+#include "content/public/browser/storage_partition.h"
+#include "extensions/browser/disable_reason.h"
+#include "extensions/browser/extension_prefs.h"
+#include "extensions/browser/extension_registrar.h"
+#include "extensions/browser/extension_registry.h"
+#include "extensions/browser/uninstall_reason.h"
+#include "extensions/common/extension.h"
+#include "net/base/load_flags.h"
+#include "net/traffic_annotation/network_traffic_annotation.h"
+#include "services/network/public/cpp/resource_request.h"
+#include "services/network/public/cpp/simple_url_loader.h"
+#include "url/url_constants.h"
+
+namespace browseros {
+
+namespace {
+
+constexpr size_t kMaxConfigBytes = 1024 * 1024;
+constexpr base::TimeDelta kConfigTimeout = base::Seconds(15);
+
+constexpr net::NetworkTrafficAnnotationTag kTrafficAnnotation =
+    net::DefineNetworkTrafficAnnotation("browseros_extension_maintenance", R"(
+        semantics {
+          sender: "BrowserOS Extension Maintainer"
+          description:
+            "Fetches JSON configuration for BrowserOS extension maintenance."
+          trigger:
+            "Background maintenance or recovery when no usable local primary "
+            "extension exists. Never blocks external provider discovery."
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
+void BrowserOSExtensionMaintainer::CheckForUpdates(const GURL& config_url,
+                                                   UpdateCallback callback) {
+  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+  callbacks_.push_back(std::move(callback));
+  if (url_loader_) {
+    return;
+  }
+
+  if (!config_url.is_valid() || !config_url.SchemeIs(url::kHttpsScheme)) {
+    Complete(base::DictValue());
+    return;
+  }
+
+  if (!url_loader_factory_) {
+    url_loader_factory_ = profile_->GetDefaultStoragePartition()
+                              ->GetURLLoaderFactoryForBrowserProcess();
+  }
+
+  auto request = std::make_unique<network::ResourceRequest>();
+  request->url = config_url;
+  request->load_flags = net::LOAD_BYPASS_CACHE | net::LOAD_DISABLE_CACHE;
+  request->credentials_mode = network::mojom::CredentialsMode::kOmit;
+  url_loader_ =
+      network::SimpleURLLoader::Create(std::move(request), kTrafficAnnotation);
+
+  // Remote discovery has its own bound and lifetime. Cancelling the owning
+  // loader drops this request; no network completion can revive a timed-out
+  // startup attempt or keep external-provider readiness pending.
+  url_loader_->SetTimeoutDuration(kConfigTimeout);
+  url_loader_->DownloadToString(
+      url_loader_factory_.get(),
+      base::BindOnce(&BrowserOSExtensionMaintainer::OnConfigFetched,
+                     weak_ptr_factory_.GetWeakPtr()),
+      kMaxConfigBytes);
+}
+
+void BrowserOSExtensionMaintainer::Cancel() {
+  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+  weak_ptr_factory_.InvalidateWeakPtrs();
+  url_loader_.reset();
+  callbacks_.clear();
+}
+
+void BrowserOSExtensionMaintainer::OnConfigFetched(
+    std::optional<std::string> response_body) {
+  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+  if (!response_body) {
+    LOG(WARNING) << "browseros: Remote extension metadata unavailable";
+    Complete(base::DictValue());
+    return;
+  }
+  Complete(ParseConfigJson(*response_body));
+}
+
+// static
+base::DictValue BrowserOSExtensionMaintainer::ParseConfigJson(
+    const std::string& json_content) {
+  std::optional<base::Value> parsed =
+      base::JSONReader::Read(json_content, base::JSON_PARSE_RFC);
+  if (!parsed || !parsed->is_dict()) {
+    return base::DictValue();
+  }
+  const base::DictValue* configs = parsed->GetDict().FindDict("extensions");
+  if (!configs) {
+    return base::DictValue();
+  }
+
+  base::DictValue prefs;
+  for (const auto [id, config] : *configs) {
+    if (!IsActiveBrowserOSExtension(id) || !config.is_dict()) {
+      continue;
+    }
+    const std::string* update_url = config.GetDict().FindString(
+        extensions::ExternalProviderImpl::kExternalUpdateUrl);
+    if (!update_url) {
+      continue;
+    }
+    const GURL url(*update_url);
+    if (!url.is_valid() || !url.SchemeIs(url::kHttpsScheme) ||
+        url.has_username() || url.has_password()) {
+      continue;
+    }
+
+    // Remote metadata can only nominate an update URL. Local file paths and
+    // version claims from this response cannot bypass Chromium's signed updater
+    // or replace the bundle's version floor.
+    base::DictValue entry;
+    entry.Set(extensions::ExternalProviderImpl::kExternalUpdateUrl, url.spec());
+    prefs.Set(id, std::move(entry));
+  }
+  return prefs;
+}
+
+void BrowserOSExtensionMaintainer::Complete(base::DictValue prefs) {
+  url_loader_.reset();
+  // Move replies out before invoking user code: a callback may Retry or destroy
+  // the coordinator and this helper. Joined callers receive the same result.
+  std::vector<UpdateCallback> callbacks = std::move(callbacks_);
+  callbacks_.clear();
+  for (auto& callback : callbacks) {
+    std::move(callback).Run(prefs.Clone());
+  }
+}
+
+void BrowserOSExtensionMaintainer::MaintainInstalledExtensions() {
+  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+  UninstallInactiveProductExtensions();
+  ReenableDisabledExtensions();
+  LogExtensionHealth("maintenance");
+}
+
+void BrowserOSExtensionMaintainer::UninstallInactiveProductExtensions() {
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
+  for (const std::string& id : GetAllBrowserOSExtensionIds()) {
+    if (IsActiveBrowserOSExtension(id)) {
+      continue;
+    }
+
+    const extensions::Extension* ext = registry->GetInstalledExtension(id);
+    if (!ext) {
+      continue;
+    }
+
+    LOG(INFO) << "browseros: Uninstalling inactive product extension " << id;
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
+void BrowserOSExtensionMaintainer::ReenableDisabledExtensions() {
+  auto* registry = extensions::ExtensionRegistry::Get(profile_);
+  auto* registrar = extensions::ExtensionRegistrar::Get(profile_);
+  for (const auto& id : GetActiveBrowserOSExtensionIds()) {
+    if (!registry->disabled_extensions().Contains(id)) {
+      continue;
+    }
+    // Keep product extensions usable, but never clear corruption, policy or
+    // other safety disablement just because a maintenance timer fired.
+    const auto reasons =
+        extensions::ExtensionPrefs::Get(profile_)->GetDisableReasons(id);
+    bool can_reenable = true;
+    for (auto reason : reasons) {
+      if (reason != extensions::disable_reason::DISABLE_USER_ACTION &&
+          reason != extensions::disable_reason::DISABLE_EXTERNAL_EXTENSION &&
+          reason != extensions::disable_reason::DISABLE_PERMISSIONS_INCREASE) {
+        can_reenable = false;
+        break;
+      }
+    }
+    if (can_reenable) {
+      LOG(INFO) << "browseros: Re-enabling managed extension " << id;
+      registrar->EnableExtension(id);
+    }
+  }
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
+  extensions::ExtensionPrefs* prefs = extensions::ExtensionPrefs::Get(profile_);
+
+  if (!registry || !prefs) {
+    return;
+  }
+
+  for (const std::string& id : GetActiveBrowserOSExtensionIds()) {
+    if (registry->enabled_extensions().Contains(id)) {
+      continue;
+    }
+
+    std::string state;
+    base::DictValue properties;
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
