diff --git a/chrome/browser/browseros/extensions/browseros_extension_loader.cc b/chrome/browser/browseros/extensions/browseros_extension_loader.cc
new file mode 100644
index 0000000000000000000000000000000000000000..f687bc2c1a5353117d53dfbd56e85ef95c32d6b8
--- /dev/null
+++ b/chrome/browser/browseros/extensions/browseros_extension_loader.cc
@@ -0,0 +1,456 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/extensions/browseros_extension_loader.h"
+
+#include <utility>
+
+#include "base/feature_list.h"
+#include "base/functional/bind.h"
+#include "base/logging.h"
+#include "base/supports_user_data.h"
+#include "base/task/single_thread_task_runner.h"
+#include "base/version.h"
+#include "chrome/browser/browser_features.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
+#include "chrome/browser/extensions/external_provider_impl.h"
+#include "chrome/browser/profiles/profile.h"
+#include "content/public/browser/browser_thread.h"
+#include "extensions/browser/disable_reason.h"
+#include "extensions/browser/extension_prefs.h"
+#include "extensions/browser/extension_registrar.h"
+#include "extensions/browser/extensions_browser_client.h"
+#include "extensions/browser/pending_extension_manager.h"
+#include "extensions/common/extension.h"
+
+namespace browseros {
+namespace {
+
+constexpr char kLoaderReferenceKey[] = "browseros.extension_loader";
+constexpr base::TimeDelta kReadinessTimeout = base::Seconds(45);
+constexpr base::TimeDelta kMaintenanceInterval = base::Minutes(15);
+
+// The external provider retains ownership. Profile data is only a weak lookup
+// for native callers, and never extends a provider or profile's lifetime.
+struct LoaderReference : public base::SupportsUserData::Data {
+  explicit LoaderReference(base::WeakPtr<BrowserOSExtensionLoader> loader)
+      : loader(std::move(loader)) {}
+  base::WeakPtr<BrowserOSExtensionLoader> loader;
+};
+
+void PostResult(BrowserOSExtensionLoader::ReadyCallback callback, bool ready) {
+  base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
+      FROM_HERE, base::BindOnce(std::move(callback), ready));
+}
+
+}  // namespace
+
+BrowserOSExtensionLoader::BrowserOSExtensionLoader(Profile* profile)
+    : profile_(profile),
+      config_url_(
+          base::FeatureList::IsEnabled(features::kBrowserOsAlphaFeatures)
+              ? kBrowserOSAlphaConfigUrl
+              : kBrowserOSConfigUrl),
+      primary_id_(IsBrowserClawProduct() ? kBrowserClawExtensionId
+                                         : kAgentExtensionId),
+      installer_(std::make_unique<BrowserOSExtensionInstaller>()),
+      maintainer_(std::make_unique<BrowserOSExtensionMaintainer>(profile)) {
+  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+  profile_->SetUserData(
+      kLoaderReferenceKey,
+      std::make_unique<LoaderReference>(weak_ptr_factory_.GetWeakPtr()));
+  profile_observation_.Observe(profile_);
+}
+
+BrowserOSExtensionLoader::~BrowserOSExtensionLoader() {
+  Shutdown();
+}
+
+// static
+void BrowserOSExtensionLoader::EnsurePrimaryExtensionReady(
+    Profile* profile,
+    ReadyCallback callback) {
+  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+  auto* reference = static_cast<LoaderReference*>(
+      profile->GetOriginalProfile()->GetUserData(kLoaderReferenceKey));
+  if (!reference || !reference->loader) {
+    PostResult(std::move(callback), false);
+    return;
+  }
+  reference->loader->EnsureReady(std::move(callback));
+}
+
+void BrowserOSExtensionLoader::SetConfigUrl(const GURL& url) {
+  config_url_ = url;
+}
+
+void BrowserOSExtensionLoader::StartLoading() {
+  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+  if (!profile_) {
+    return;
+  }
+  if (local_discovery_complete_) {
+    // Each provider visit expects its metadata callback, even after discovery.
+    // Replay the retained snapshot; Chromium deduplicates pending installs.
+    LoadFinished(BuildDesiredPrefs());
+    return;
+  }
+  if (started_) {
+    return;
+  }
+  // Chromium may ask the provider to load repeatedly. Never replace helpers:
+  // their weak replies belong to this single discovery operation.
+  started_ = true;
+  installer_->DiscoverBundled(
+      base::BindOnce(&BrowserOSExtensionLoader::OnLocalDiscoveryComplete,
+                     weak_ptr_factory_.GetWeakPtr()));
+}
+
+void BrowserOSExtensionLoader::EnsureReady(ReadyCallback callback) {
+  if (!profile_) {
+    PostResult(std::move(callback), false);
+    return;
+  }
+  if (local_discovery_complete_ && IsPrimaryReady()) {
+    PostResult(std::move(callback), true);
+    return;
+  }
+  ready_callbacks_.push_back(std::move(callback));
+  if (waiting_for_ready_) {
+    return;
+  }
+  // Only a new attempt resets failed local metadata. Calls during an attempt
+  // join its observation, timeout and install; they cannot queue another CRX.
+  const auto* pending =
+      extensions::PendingExtensionManager::Get(profile_)->GetById(primary_id_);
+  // Failed downloads retain a URL pending record. Chromium will reject a
+  // same-source local file while that record exists, so Retry must reannounce
+  // the URL to the updater rather than silently waiting on an unqueued file.
+  if (!pending || pending->update_url().is_empty()) {
+    failed_bundles_.erase(primary_id_);
+  }
+  BeginReadiness();
+  if (!started_) {
+    StartLoading();
+  }
+  if (local_discovery_complete_) {
+    PublishUpdates();
+    AdvanceReadiness();
+  }
+}
+
+void BrowserOSExtensionLoader::OnLocalDiscoveryComplete(
+    BundleDiscoveryResult result) {
+  bundled_crx_base_path_ = std::move(result.bundled_path);
+  bundled_prefs_ = std::move(result.prefs);
+  local_discovery_complete_ = true;
+  LOG(INFO) << "browseros: Local extension discovery complete: "
+            << bundled_prefs_.size()
+            << " entries, complete=" << result.complete;
+
+  BeginReadiness();
+  // LoadFinished acknowledges metadata, NOT successful installation. Even an
+  // empty/partial bundle must release Chromium's external-provider barrier.
+  LoadFinished(BuildDesiredPrefs());
+  // Updates can call the updater, which is not alive during ExtensionService
+  // initialization. Cross that startup seam before any recovery/maintenance.
+  base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
+      FROM_HERE, base::BindOnce(&BrowserOSExtensionLoader::AdvanceReadiness,
+                                weak_ptr_factory_.GetWeakPtr()));
+}
+
+const base::FilePath BrowserOSExtensionLoader::GetBaseCrxFilePath() {
+  return bundled_crx_base_path_;
+}
+
+bool BrowserOSExtensionLoader::NeedsLocalInstall(const std::string& id) const {
+  const base::DictValue* local = bundled_prefs_.FindDict(id);
+  if (!local || failed_bundles_.contains(id)) {
+    return false;
+  }
+  const auto* installed =
+      extensions::ExtensionRegistry::Get(profile_)->GetInstalledExtension(id);
+  const base::Version version(
+      *local->FindString(extensions::ExternalProviderImpl::kExternalVersion));
+  return !installed || installed->version().CompareTo(version) < 0;
+}
+
+bool BrowserOSExtensionLoader::HasUsableLocalPrimary() const {
+  auto* registry = extensions::ExtensionRegistry::Get(profile_);
+  return (bundled_prefs_.contains(primary_id_) &&
+          !failed_bundles_.contains(primary_id_)) ||
+         registry->enabled_extensions().Contains(primary_id_) ||
+         registry->terminated_extensions().Contains(primary_id_);
+}
+
+bool BrowserOSExtensionLoader::IsPrimaryReady() const {
+  if (!profile_ || !local_discovery_complete_) {
+    return false;
+  }
+  return extensions::ExtensionRegistry::Get(profile_)
+             ->ready_extensions()
+             .Contains(primary_id_) &&
+         !NeedsLocalInstall(primary_id_);
+}
+
+base::DictValue BrowserOSExtensionLoader::BuildDesiredPrefs() const {
+  base::DictValue prefs;
+  auto* registry = extensions::ExtensionRegistry::Get(profile_);
+  for (const std::string& id : GetActiveBrowserOSExtensionIds()) {
+    const auto* local = bundled_prefs_.FindDict(id);
+    const auto* remote = remote_prefs_.FindDict(id);
+    if (NeedsLocalInstall(id)) {
+      prefs.Set(id, local->Clone());
+    } else if (remote) {
+      prefs.Set(id, remote->Clone());
+    } else if (local && !failed_bundles_.contains(id)) {
+      prefs.Set(id, local->Clone());
+    } else if (registry->GetInstalledExtension(id)) {
+      // Retain installed active extensions when bundle metadata is absent.
+      // Omitting them would make external-provider orphan cleanup uninstall
+      // them. Chromium skips URL installs for an already installed extension.
+      base::DictValue retained;
+      retained.Set(
+          extensions::ExternalProviderImpl::kExternalUpdateUrl,
+          base::FeatureList::IsEnabled(features::kBrowserOsAlphaFeatures)
+              ? kBrowserOSAlphaUpdateUrl
+              : kBrowserOSUpdateUrl);
+      prefs.Set(id, std::move(retained));
+    }
+  }
+  return prefs;
+}
+
+void BrowserOSExtensionLoader::BeginReadiness() {
+  if (waiting_for_ready_ || !profile_) {
+    return;
+  }
+  waiting_for_ready_ = true;
+  remote_requested_ = false;
+  if (!install_observation_.IsObserving()) {
+    // Secondary local installs can finish after the primary is READY. Track
+    // their failures throughout maintenance so a bad bundle can fall back to
+    // the remote provider instead of being retried forever.
+    install_observation_.Observe(
+        extensions::ExtensionsBrowserClient::Get()->GetInstallTracker(
+            profile_));
+  }
+  if (IsPrimaryReady()) {
+    FinishReadiness(true);
+    return;
+  }
+  // Registry access and registration share the UI sequence. Always inspect
+  // the READY set as well as observing: installed profiles may already be
+  // ready.
+  registry_observation_.Observe(extensions::ExtensionRegistry::Get(profile_));
+  readiness_timer_.Start(
+      FROM_HERE, kReadinessTimeout,
+      base::BindOnce(&BrowserOSExtensionLoader::OnReadinessTimeout,
+                     weak_ptr_factory_.GetWeakPtr()));
+}
+
+void BrowserOSExtensionLoader::RestorePrimaryExtension() {
+  auto* registry = extensions::ExtensionRegistry::Get(profile_);
+  auto* registrar = extensions::ExtensionRegistrar::Get(profile_);
+  if (registry->terminated_extensions().Contains(primary_id_)) {
+    registrar->ReloadExtension(primary_id_);
+  } else if (registry->disabled_extensions().Contains(primary_id_)) {
+    // Recover user/prompt disablement of a managed product extension, but do
+    // not clear corruption, policy or other safety-related disable reasons.
+    const auto reasons =
+        extensions::ExtensionPrefs::Get(profile_)->GetDisableReasons(
+            primary_id_);
+    for (auto reason : reasons) {
+      if (reason != extensions::disable_reason::DISABLE_USER_ACTION &&
+          reason != extensions::disable_reason::DISABLE_EXTERNAL_EXTENSION &&
+          reason != extensions::disable_reason::DISABLE_PERMISSIONS_INCREASE) {
+        return;
+      }
+    }
+    registrar->EnableExtension(primary_id_);
+  }
+}
+
+void BrowserOSExtensionLoader::AdvanceReadiness() {
+  if (!profile_ || !waiting_for_ready_ || !local_discovery_complete_) {
+    return;
+  }
+  if (IsPrimaryReady()) {
+    FinishReadiness(true);
+    return;
+  }
+  RestorePrimaryExtension();
+  if (!waiting_for_ready_) {
+    return;  // Enabling can synchronously notify READY.
+  }
+  if (IsPrimaryReady()) {
+    FinishReadiness(true);
+    return;
+  }
+  if (!HasUsableLocalPrimary() && !remote_requested_) {
+    // Config fetches never delay LoadFinished. Only the absence of a usable
+    // local primary permits recovery before READY; all other network work is
+    // background maintenance after a successful registry result.
+    CheckForUpdates();
+  }
+}
+
+void BrowserOSExtensionLoader::PublishUpdates() {
+  if (profile_ && local_discovery_complete_ && has_owner()) {
+    // Full desired-state snapshots preserve ownership of every active ID.
+    // Chromium deduplicates pending files and refuses same/older versions.
+    OnUpdated(BuildDesiredPrefs());
+  }
+}
+
+void BrowserOSExtensionLoader::CheckForUpdates() {
+  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
+  if (!profile_ || !local_discovery_complete_ || remote_requested_ ||
+      (!maintenance_started_ && !waiting_for_ready_) ||
+      (waiting_for_ready_ && HasUsableLocalPrimary())) {
+    return;
+  }
+  remote_requested_ = true;
+  maintainer_->CheckForUpdates(
+      config_url_,
+      base::BindOnce(&BrowserOSExtensionLoader::OnRemoteConfigLoaded,
+                     weak_ptr_factory_.GetWeakPtr()));
+}
+
+void BrowserOSExtensionLoader::OnRemoteConfigLoaded(base::DictValue prefs) {
+  // Preserve the last valid remote entry when a config is partial. Neither
+  // network failure nor a missing config member revokes product ownership.
+  for (auto [id, pref] : prefs) {
+    remote_prefs_.Set(id, pref.Clone());
+  }
+  if (waiting_for_ready_ && !HasUsableLocalPrimary() &&
+      !remote_prefs_.contains(primary_id_)) {
+    FinishReadiness(false);
+    return;
+  }
+  PublishUpdates();
+  if (maintenance_started_ && !waiting_for_ready_) {
+    // OnUpdated already asks Chromium's updater to check both pending and
+    // installed extensions. Do not enqueue a second CheckNow here. Installed
+    // extensions use their manifest URL and only accept a newer version.
+    remote_requested_ = false;
+  }
+}
+
+void BrowserOSExtensionLoader::FinishReadiness(bool ready) {
+  if (!waiting_for_ready_) {
+    return;
+  }
+  waiting_for_ready_ = false;
+  readiness_timer_.Stop();
+  registry_observation_.Reset();
+  maintainer_->Cancel();
+  remote_requested_ = false;
+  LOG(INFO) << "browseros: Primary extension readiness "
+            << (ready ? "READY" : "failed; retry available");
+  auto callbacks = std::move(ready_callbacks_);
+  ready_callbacks_.clear();
+  // Post results rather than destroying the onboarding window inside a
+  // registry notification. Weak callbacks at the UI boundary cancel safely.
+  for (auto& callback : callbacks) {
+    PostResult(std::move(callback), ready);
+  }
+  if (ready && !maintenance_started_) {
+    maintenance_started_ = true;
+    maintenance_timer_.Start(
+        FROM_HERE, kMaintenanceInterval, this,
+        &BrowserOSExtensionLoader::MaintainInstalledExtensions);
+    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
+        FROM_HERE,
+        base::BindOnce(&BrowserOSExtensionLoader::MaintainInstalledExtensions,
+                       weak_ptr_factory_.GetWeakPtr()));
+  }
+}
+
+void BrowserOSExtensionLoader::MaintainInstalledExtensions() {
+  if (!profile_ || waiting_for_ready_) {
+    return;
+  }
+  // Keep local housekeeping independent of network success. The coordinator
+  // schedules both halves; the helper cannot initiate an overlapping install.
+  maintainer_->MaintainInstalledExtensions();
+  CheckForUpdates();
+}
+
+void BrowserOSExtensionLoader::OnReadinessTimeout() {
+  // This is a UX bound, never evidence that installation succeeded. Chromium
+  // may still finish its own CRX work; Retry joins that work through the
+  // provider.
+  FinishReadiness(false);
+}
+
+void BrowserOSExtensionLoader::OnExtensionReady(
+    content::BrowserContext* context,
+    const extensions::Extension* extension) {
+  if (extension->id() == primary_id_ && IsPrimaryReady()) {
+    FinishReadiness(true);
+  }
+}
+
+void BrowserOSExtensionLoader::OnFinishCrxInstall(
+    content::BrowserContext* context,
+    const base::FilePath& source_file,
+    const std::string& extension_id,
+    const extensions::Extension* extension,
+    bool success) {
+  if (success || !IsActiveBrowserOSExtension(extension_id)) {
+    return;
+  }
+  const auto* local = bundled_prefs_.FindDict(extension_id);
+  if (local &&
+      source_file.AsUTF8Unsafe() ==
+          *local->FindString(extensions::ExternalProviderImpl::kExternalCrx)) {
+    failed_bundles_.insert(extension_id);
+  } else if (extension_id == primary_id_) {
+    FinishReadiness(false);
+    return;
+  }
+  // The failed pending record is removed by a posted installer callback.
+  // Recovery fetches metadata asynchronously before publishing the URL, so
+  // that stale file record cannot suppress the subsequent provider update.
+  base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
+      FROM_HERE,
+      base::BindOnce(waiting_for_ready_
+                         ? &BrowserOSExtensionLoader::AdvanceReadiness
+                         : &BrowserOSExtensionLoader::CheckForUpdates,
+                     weak_ptr_factory_.GetWeakPtr()));
+}
+
+void BrowserOSExtensionLoader::OnShutdown(
+    extensions::ExtensionRegistry* registry) {
+  Shutdown();
+}
+
+void BrowserOSExtensionLoader::OnShutdown() {
+  Shutdown();
+}
+
+void BrowserOSExtensionLoader::OnProfileWillBeDestroyed(Profile* profile) {
+  Shutdown();
+}
+
+void BrowserOSExtensionLoader::Shutdown() {
+  if (!profile_) {
+    return;
+  }
+  // Cancel replies and observations while profile services are still alive.
+  // ExternalLoader itself may outlive them through the provider's references.
+  weak_ptr_factory_.InvalidateWeakPtrs();
+  FinishReadiness(false);
+  maintenance_timer_.Stop();
+  maintainer_.reset();
+  installer_.reset();
+  registry_observation_.Reset();
+  install_observation_.Reset();
+  profile_observation_.Reset();
+  profile_->RemoveUserData(kLoaderReferenceKey);
+  profile_ = nullptr;
+}
+
+}  // namespace browseros
