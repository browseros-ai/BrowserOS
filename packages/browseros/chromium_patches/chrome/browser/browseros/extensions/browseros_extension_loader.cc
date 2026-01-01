diff --git a/chrome/browser/browseros/extensions/browseros_extension_loader.cc b/chrome/browser/browseros/extensions/browseros_extension_loader.cc
new file mode 100644
index 0000000000000..3bc7b4f19f999
--- /dev/null
+++ b/chrome/browser/browseros/extensions/browseros_extension_loader.cc
@@ -0,0 +1,151 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/extensions/browseros_extension_loader.h"
+
+#include <utility>
+
+#include "base/feature_list.h"
+#include "base/logging.h"
+#include "base/task/single_thread_task_runner.h"
+#include "chrome/browser/browser_features.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
+#include "chrome/browser/extensions/external_provider_impl.h"
+#include "chrome/browser/extensions/updater/extension_updater.h"
+#include "chrome/browser/profiles/profile.h"
+#include "extensions/browser/extension_registry.h"
+#include "extensions/browser/pending_extension_manager.h"
+#include "extensions/common/extension.h"
+#include "extensions/common/mojom/manifest.mojom-shared.h"
+
+namespace browseros {
+
+namespace {
+
+constexpr base::TimeDelta kImmediateInstallDelay = base::Seconds(2);
+
+}  // namespace
+
+BrowserOSExtensionLoader::BrowserOSExtensionLoader(Profile* profile)
+    : profile_(profile) {
+  config_url_ =
+      GURL(base::FeatureList::IsEnabled(features::kBrowserOsAlphaFeatures)
+               ? kBrowserOSAlphaConfigUrl
+               : kBrowserOSConfigUrl);
+
+  for (const std::string& id : GetBrowserOSExtensionIds()) {
+    extension_ids_.insert(id);
+  }
+}
+
+BrowserOSExtensionLoader::~BrowserOSExtensionLoader() = default;
+
+void BrowserOSExtensionLoader::SetConfigUrl(const GURL& url) {
+  config_url_ = url;
+}
+
+void BrowserOSExtensionLoader::StartLoading() {
+  LOG(INFO) << "browseros: Extension loader starting";
+
+  installer_ = std::make_unique<BrowserOSExtensionInstaller>(profile_);
+  maintainer_ = std::make_unique<BrowserOSExtensionMaintainer>(profile_);
+
+  installer_->StartInstallation(
+      config_url_,
+      base::BindOnce(&BrowserOSExtensionLoader::OnInstallComplete,
+                     weak_ptr_factory_.GetWeakPtr()));
+}
+
+void BrowserOSExtensionLoader::OnInstallComplete(InstallResult result) {
+  if (result.from_bundled) {
+    bundled_crx_base_path_ = result.bundled_path;
+  }
+
+  extension_ids_.merge(result.extension_ids);
+  last_config_ = std::move(result.config);
+
+  LOG(INFO) << "browseros: Install complete, " << result.prefs.size()
+            << " extensions (from_bundled=" << result.from_bundled << ")";
+
+  LoadFinished(std::move(result.prefs));
+  OnStartupComplete(result.from_bundled);
+}
+
+const base::FilePath BrowserOSExtensionLoader::GetBaseCrxFilePath() {
+  return bundled_crx_base_path_;
+}
+
+void BrowserOSExtensionLoader::OnStartupComplete(bool from_bundled) {
+  LOG(INFO) << "browseros: Startup complete (from_bundled=" << from_bundled
+            << ")";
+
+  if (!from_bundled) {
+    base::SingleThreadTaskRunner::GetCurrentDefault()->PostDelayedTask(
+        FROM_HERE,
+        base::BindOnce(&BrowserOSExtensionLoader::TriggerImmediateInstallation,
+                       weak_ptr_factory_.GetWeakPtr()),
+        kImmediateInstallDelay);
+  }
+
+  maintainer_->Start(config_url_, extension_ids_, std::move(last_config_));
+}
+
+void BrowserOSExtensionLoader::TriggerImmediateInstallation() {
+  if (!profile_ || extension_ids_.empty()) {
+    return;
+  }
+
+  extensions::ExtensionRegistry* registry =
+      extensions::ExtensionRegistry::Get(profile_);
+  extensions::PendingExtensionManager* pending =
+      extensions::PendingExtensionManager::Get(profile_);
+
+  if (!registry || !pending || last_config_.empty()) {
+    return;
+  }
+
+  LOG(INFO) << "browseros: Triggering immediate installation";
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
+    pending->AddFromExternalUpdateUrl(
+        id, std::string(), url,
+        extensions::mojom::ManifestLocation::kExternalComponent,
+        extensions::Extension::WAS_INSTALLED_BY_DEFAULT, false);
+
+    LOG(INFO) << "browseros: Added " << id << " to pending";
+  }
+
+  extensions::ExtensionUpdater* updater =
+      extensions::ExtensionUpdater::Get(profile_);
+  if (updater) {
+    extensions::ExtensionUpdater::CheckParams params;
+    params.ids = std::list<extensions::ExtensionId>(extension_ids_.begin(),
+                                                     extension_ids_.end());
+    params.install_immediately = true;
+    params.fetch_priority = extensions::DownloadFetchPriority::kForeground;
+    updater->CheckNow(std::move(params));
+  }
+}
+
+}  // namespace browseros
