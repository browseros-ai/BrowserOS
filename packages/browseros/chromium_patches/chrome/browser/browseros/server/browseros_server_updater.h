diff --git a/chrome/browser/browseros/server/browseros_server_updater.h b/chrome/browser/browseros/server/browseros_server_updater.h
new file mode 100644
index 0000000000000..022b703a32e8d
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_updater.h
@@ -0,0 +1,164 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_UPDATER_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_UPDATER_H_
+
+#include <memory>
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
+}
+
+namespace browseros_server {
+
+// Manages automatic updates for the BrowserOS server binary.
+//
+// Update flow:
+// 1. Fetch appcast XML from CDN
+// 2. Parse and find matching platform enclosure
+// 3. Download ZIP if newer version available
+// 4. Verify Ed25519 signature
+// 5. Extract to versions/{version}/
+// 6. Test binary with --version
+// 7. Update current_version file
+// 8. Signal manager to use new binary on next restart
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
+  void OnAppcastFetched(std::unique_ptr<std::string> response);
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
+  // Binary testing
+  void TestBinary(const base::Version& version);
+  void OnBinaryTestComplete(const base::Version& version,
+                            int exit_code,
+                            const std::string& output);
+
+  // Hot-swap flow
+  void CheckServerStatus();
+  void OnStatusFetched(std::unique_ptr<std::string> response);
+  void OnServerStatusChecked(bool can_update);
+  void PerformHotSwap(const base::Version& version);
+  void OnHotSwapComplete(const base::Version& old_version,
+                         const base::Version& new_version,
+                         bool success);
+
+  // Version management
+  base::Version GetCurrentVersion();
+  base::Version GetBundledVersion();
+  base::Version GetLatestDownloadedVersion();
+  void LoadVersionCachesAsync();
+  void OnDownloadedVersionLoaded(const std::string& version_str);
+  void OnBundledVersionLoaded(int exit_code, const std::string& output);
+  void CheckVersionCachesAndStart();
+  void WriteCurrentVersionFile(const base::Version& version);
+
+  // Path helpers
+  base::FilePath GetExecutionDir() const;
+  base::FilePath GetVersionsDir() const;
+  base::FilePath GetVersionDir(const base::Version& version) const;
+  base::FilePath GetPendingUpdateDir() const;
+  base::FilePath GetBundledBinaryPath() const;
+  base::FilePath GetBundledResourcesPath() const;
+  base::FilePath GetDownloadedBinaryPath(const base::Version& version) const;
+  base::FilePath GetDownloadedResourcesPath(const base::Version& version) const;
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
+
+  base::RepeatingTimer update_check_timer_;
+
+  State state_ = State::kIdle;
+  bool update_in_progress_ = false;
+
+  // Keep loaders alive during async operations
+  std::unique_ptr<network::SimpleURLLoader> appcast_loader_;
+  std::unique_ptr<network::SimpleURLLoader> download_loader_;
+  std::unique_ptr<network::SimpleURLLoader> status_loader_;
+
+  // Pending update info
+  AppcastItem pending_item_;
+  std::string pending_signature_;
+
+  // Cached versions (loaded async at startup via --version)
+  base::Version cached_bundled_version_;
+  base::Version cached_downloaded_version_;
+  bool bundled_version_loaded_ = false;
+  bool downloaded_version_loaded_ = false;
+
+  base::WeakPtrFactory<BrowserOSServerUpdater> weak_factory_{this};
+};
+
+}  // namespace browseros_server
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_UPDATER_H_
