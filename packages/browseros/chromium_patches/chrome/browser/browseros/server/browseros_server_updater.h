diff --git a/chrome/browser/browseros/server/browseros_server_updater.h b/chrome/browser/browseros/server/browseros_server_updater.h
new file mode 100644
index 0000000000000000000000000000000000000000..72411a77f7ef7345280a8c8f5b53d2348978862f
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_updater.h
@@ -0,0 +1,140 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_UPDATER_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_UPDATER_H_
+
+#include <memory>
+#include <optional>
+#include <string>
+
+#include "base/files/file_path.h"
+#include "base/memory/raw_ptr.h"
+#include "base/memory/weak_ptr.h"
+#include "base/timer/timer.h"
+#include "base/version.h"
+#include "chrome/browser/browseros/server/browseros_appcast_parser.h"
+#include "chrome/browser/browseros/server/server_updater.h"
+
+namespace network {
+class SimpleURLLoader;
+}
+
+namespace browseros {
+class BrowserOSServerManager;
+class ServerVersionStore;
+struct ManagedServerDescriptor;
+}  // namespace browseros
+
+namespace browseros_server {
+
+// Manages automatic updates for the selected BrowserOS sidecar server.
+//
+// Update flow:
+// 1. Fetch the descriptor-selected appcast XML from CDN.
+// 2. Parse and find the matching platform enclosure.
+// 3. Download ZIP if a newer version is available.
+// 4. Verify Ed25519 signature.
+// 5. Extract to the product's versions/{version}/ directory.
+// 6. Ask the version store to validate and persist a ready installation.
+// 7. Notify the manager, which owns readiness, activation retries, and health.
+class BrowserOSServerUpdater : public browseros::ServerUpdater {
+ public:
+  explicit BrowserOSServerUpdater(browseros::BrowserOSServerManager* manager);
+  ~BrowserOSServerUpdater() override;
+
+  BrowserOSServerUpdater(const BrowserOSServerUpdater&) = delete;
+  BrowserOSServerUpdater& operator=(const BrowserOSServerUpdater&) = delete;
+
+  // ServerUpdater implementation:
+  void Start() override;
+  void Stop() override;
+  bool IsUpdateInProgress() const override;
+  base::FilePath GetBestServerBinaryPath() override;
+  base::FilePath GetBestServerResourcesPath() override;
+  void InvalidateDownloadedVersion() override;
+  void OnServerActivated() override;
+
+  // Forces an immediate update check (not part of interface).
+  void CheckNow();
+
+ private:
+  enum class State {
+    kIdle,
+    kFetchingAppcast,
+    kDownloading,
+    kVerifying,
+    kExtracting,
+    kTesting,
+  };
+
+  void OnUpdateTimer();
+
+  // Appcast flow
+  void FetchAppcast();
+  void OnAppcastFetched(std::optional<std::string> response);
+
+  // Download flow
+  void CheckVersionAlreadyDownloaded(const AppcastEnclosure& enclosure,
+                                     const base::Version& version);
+  void OnVersionExistsCheck(const AppcastEnclosure& enclosure,
+                            const base::Version& version,
+                            bool exists);
+  void StartDownload(const AppcastEnclosure& enclosure,
+                     const base::Version& version);
+  void OnDownloadComplete(const base::Version& version,
+                          base::FilePath zip_path);
+
+  // Verification flow (runs on background thread)
+  void VerifyAndExtract(const base::FilePath& zip_path,
+                        const std::string& signature,
+                        const base::Version& version);
+  void OnVerifyAndExtractComplete(const base::Version& version,
+                                  bool success,
+                                  const std::string& error);
+
+  // Local preparation delegates probing and persistence to the version store.
+  void TestBinary(const base::Version& version);
+  void OnVersionPrepared(const base::Version& version, bool success);
+
+  // Path helpers
+  base::FilePath GetVersionsDir() const;
+  base::FilePath GetVersionDir(const base::Version& version) const;
+  base::FilePath GetPendingUpdateDir() const;
+
+  // Cleanup
+  void CleanupPendingUpdate();
+  void CleanupOldVersions();
+
+  // Error handling
+  void OnError(const std::string& stage, const std::string& error);
+  void ResetState();
+
+  raw_ptr<browseros::BrowserOSServerManager> manager_;
+  // Manager-owned; polling stops before the local catalog is destroyed.
+  raw_ptr<browseros::ServerVersionStore> version_store_;
+
+  // Selected sidecar's OTA contract (appcast feeds, state dir, binary name,
+  // readiness path). Points at a process-lifetime static descriptor.
+  raw_ptr<const browseros::ManagedServerDescriptor> descriptor_;
+
+  base::RepeatingTimer update_check_timer_;
+
+  State state_ = State::kIdle;
+  bool update_in_progress_ = false;
+
+  // Keep loaders alive during async operations
+  std::unique_ptr<network::SimpleURLLoader> appcast_loader_;
+  std::unique_ptr<network::SimpleURLLoader> download_loader_;
+
+  // Pending update info
+  AppcastItem pending_item_;
+  std::string pending_signature_;
+
+  base::WeakPtrFactory<BrowserOSServerUpdater> weak_factory_{this};
+};
+
+}  // namespace browseros_server
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_UPDATER_H_
