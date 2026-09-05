diff --git a/chrome/browser/browseros/extensions/browseros_extension_loader.h b/chrome/browser/browseros/extensions/browseros_extension_loader.h
new file mode 100644
index 0000000000000000000000000000000000000000..14982f96c6dc7545680517da758b5b7b05ba82ef
--- /dev/null
+++ b/chrome/browser/browseros/extensions/browseros_extension_loader.h
@@ -0,0 +1,118 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_LOADER_H_
+#define CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_LOADER_H_
+
+#include <memory>
+#include <set>
+#include <string>
+#include <vector>
+
+#include "base/memory/weak_ptr.h"
+#include "base/scoped_observation.h"
+#include "base/timer/timer.h"
+#include "chrome/browser/browseros/extensions/browseros_extension_installer.h"
+#include "chrome/browser/browseros/extensions/browseros_extension_maintainer.h"
+#include "chrome/browser/extensions/external_loader.h"
+#include "chrome/browser/profiles/profile_observer.h"
+#include "extensions/browser/extension_registry.h"
+#include "extensions/browser/extension_registry_observer.h"
+#include "extensions/browser/install_observer.h"
+#include "extensions/browser/install_tracker.h"
+
+class Profile;
+
+namespace browseros {
+
+// One per-profile coordinator owns desired extension state. Discovery supplies
+// provider preferences; only Chromium's external provider executes installs.
+// Provider readiness is local metadata availability, whereas navigation waits
+// for registry READY (safe to create an extension process, even for lazy MV3).
+class BrowserOSExtensionLoader : public extensions::ExternalLoader,
+                                 public extensions::ExtensionRegistryObserver,
+                                 public extensions::InstallObserver,
+                                 public ProfileObserver {
+ public:
+  using ReadyCallback = base::OnceCallback<void(bool)>;
+
+  explicit BrowserOSExtensionLoader(Profile* profile);
+  BrowserOSExtensionLoader(const BrowserOSExtensionLoader&) = delete;
+  BrowserOSExtensionLoader& operator=(const BrowserOSExtensionLoader&) = delete;
+
+  // UI-sequence callers join the existing operation. A false result is
+  // recoverable: another call retries local publication or remote recovery.
+  // The profile must be the browsing profile, not the onboarding picker.
+  static void EnsurePrimaryExtensionReady(Profile* profile,
+                                          ReadyCallback callback);
+  void CheckForUpdates();
+  void SetConfigUrl(const GURL& url);
+
+ protected:
+  ~BrowserOSExtensionLoader() override;
+  void StartLoading() override;
+  const base::FilePath GetBaseCrxFilePath() override;
+
+ private:
+  friend class base::RefCountedThreadSafe<extensions::ExternalLoader>;
+
+  void EnsureReady(ReadyCallback callback);
+  void OnLocalDiscoveryComplete(BundleDiscoveryResult result);
+  void BeginReadiness();
+  void AdvanceReadiness();
+  void FinishReadiness(bool ready);
+  bool IsPrimaryReady() const;
+  bool HasUsableLocalPrimary() const;
+  bool NeedsLocalInstall(const std::string& id) const;
+  base::DictValue BuildDesiredPrefs() const;
+  void PublishUpdates();
+  void OnRemoteConfigLoaded(base::DictValue prefs);
+  void OnReadinessTimeout();
+  void RestorePrimaryExtension();
+  void MaintainInstalledExtensions();
+  void Shutdown();
+
+  void OnExtensionReady(content::BrowserContext* context,
+                        const extensions::Extension* extension) override;
+  void OnFinishCrxInstall(content::BrowserContext* context,
+                          const base::FilePath& source_file,
+                          const std::string& extension_id,
+                          const extensions::Extension* extension,
+                          bool success) override;
+  void OnShutdown(extensions::ExtensionRegistry* registry) override;
+  void OnShutdown() override;
+  void OnProfileWillBeDestroyed(Profile* profile) override;
+
+  raw_ptr<Profile> profile_;
+  GURL config_url_;
+  const std::string primary_id_;
+  base::FilePath bundled_crx_base_path_;
+  base::DictValue bundled_prefs_;
+  base::DictValue remote_prefs_;
+  // Failed packages remain in bundled_prefs_ for an explicit local Retry.
+  std::set<std::string> failed_bundles_;
+  bool started_ = false;
+  bool local_discovery_complete_ = false;
+  bool waiting_for_ready_ = false;
+  bool remote_requested_ = false;
+  bool maintenance_started_ = false;
+  std::vector<ReadyCallback> ready_callbacks_;
+  base::OneShotTimer readiness_timer_;
+  base::RepeatingTimer maintenance_timer_;
+
+  std::unique_ptr<BrowserOSExtensionInstaller> installer_;
+  std::unique_ptr<BrowserOSExtensionMaintainer> maintainer_;
+  base::ScopedObservation<Profile, ProfileObserver> profile_observation_{this};
+  base::ScopedObservation<extensions::ExtensionRegistry,
+                          extensions::ExtensionRegistryObserver>
+      registry_observation_{this};
+  base::ScopedObservation<extensions::InstallTracker,
+                          extensions::InstallObserver>
+      install_observation_{this};
+  base::WeakPtrFactory<BrowserOSExtensionLoader> weak_ptr_factory_{this};
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_EXTENSIONS_BROWSEROS_EXTENSION_LOADER_H_
