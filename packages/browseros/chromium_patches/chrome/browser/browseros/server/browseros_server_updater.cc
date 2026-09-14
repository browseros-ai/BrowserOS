diff --git a/chrome/browser/browseros/server/browseros_server_updater.cc b/chrome/browser/browseros/server/browseros_server_updater.cc
new file mode 100644
index 0000000000000000000000000000000000000000..7c5df6afacc045b7d34559b9d47a85481bb57ed6
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_updater.cc
@@ -0,0 +1,641 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_server_updater.h"
+
+#include <algorithm>
+#include <optional>
+#include <vector>
+
+#include "base/base64.h"
+#include "base/command_line.h"
+#include "base/feature_list.h"
+#include "base/files/file_enumerator.h"
+#include "base/files/file_util.h"
+#include "base/logging.h"
+#include "chrome/browser/browser_features.h"
+#include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_switches.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics.h"
+#include "chrome/browser/browseros/server/browseros_server_config.h"
+#include "chrome/browser/browseros/server/browseros_server_constants.h"
+#include "chrome/browser/browseros/server/browseros_server_manager.h"
+#include "chrome/browser/browseros/server/server_version_store.h"
+#include "chrome/browser/net/system_network_context_manager.h"
+#include "net/base/net_errors.h"
+#include "net/traffic_annotation/network_traffic_annotation.h"
+#include "services/network/public/cpp/resource_request.h"
+#include "services/network/public/cpp/simple_url_loader.h"
+#include "third_party/boringssl/src/include/openssl/curve25519.h"
+#include "third_party/zlib/google/zip.h"
+#include "third_party/zlib/google/zip_reader.h"
+#include "url/gurl.h"
+
+namespace browseros_server {
+
+namespace {
+
+net::NetworkTrafficAnnotationTag GetAppcastTrafficAnnotation() {
+  return net::DefineNetworkTrafficAnnotation("browseros_server_appcast", R"(
+    semantics {
+      sender: "BrowserOS Server Updater"
+      description:
+        "Checks for updates to the BrowserOS server component by fetching "
+        "an appcast XML feed."
+      trigger: "Periodic check every 15 minutes while browser is running."
+      data: "No user data sent, just an HTTP GET request."
+      destination: OTHER
+      internal {
+        contacts {
+          email: "nikhil@browseros.com"
+        }
+      }
+    }
+    policy {
+      cookies_allowed: NO
+      setting: "This feature can be disabled via --disable-browseros-server or --disable-browseros-server-updater."
+      policy_exception_justification:
+        "Essential for keeping BrowserOS server component up to date."
+    })");
+}
+
+net::NetworkTrafficAnnotationTag GetDownloadTrafficAnnotation() {
+  return net::DefineNetworkTrafficAnnotation("browseros_server_download", R"(
+    semantics {
+      sender: "BrowserOS Server Updater"
+      description:
+        "Downloads a new version of the BrowserOS server component."
+      trigger: "When a newer version is available in the appcast feed."
+      data: "No user data sent, just an HTTP GET request for the ZIP package."
+      destination: OTHER
+      internal {
+        contacts {
+          email: "nikhil@browseros.com"
+        }
+      }
+    }
+    policy {
+      cookies_allowed: NO
+      setting: "This feature can be disabled via --disable-browseros-server or --disable-browseros-server-updater."
+      policy_exception_justification:
+        "Essential for keeping BrowserOS server component up to date."
+    })");
+}
+
+// Verifies Ed25519 signature of file contents.
+// Returns true if signature is valid.
+bool VerifyEd25519Signature(const base::FilePath& file_path,
+                            const std::string& signature_base64,
+                            const std::string& public_key_base64) {
+  // Decode public key
+  std::string public_key_bytes;
+  if (!base::Base64Decode(public_key_base64, &public_key_bytes)) {
+    LOG(ERROR) << "browseros: Failed to decode public key from base64";
+    return false;
+  }
+  if (public_key_bytes.size() != ED25519_PUBLIC_KEY_LEN) {
+    LOG(ERROR) << "browseros: Invalid public key length: "
+               << public_key_bytes.size() << " (expected "
+               << ED25519_PUBLIC_KEY_LEN << ")";
+    return false;
+  }
+
+  // Decode signature
+  std::string signature_bytes;
+  if (!base::Base64Decode(signature_base64, &signature_bytes)) {
+    LOG(ERROR) << "browseros: Failed to decode signature from base64";
+    return false;
+  }
+  if (signature_bytes.size() != ED25519_SIGNATURE_LEN) {
+    LOG(ERROR) << "browseros: Invalid signature length: "
+               << signature_bytes.size() << " (expected "
+               << ED25519_SIGNATURE_LEN << ")";
+    return false;
+  }
+
+  // Read file contents
+  std::string file_contents;
+  if (!base::ReadFileToString(file_path, &file_contents)) {
+    LOG(ERROR) << "browseros: Failed to read file for signature verification: "
+               << file_path;
+    return false;
+  }
+
+  // Verify signature
+  const uint8_t* message =
+      reinterpret_cast<const uint8_t*>(file_contents.data());
+  size_t message_len = file_contents.size();
+  const uint8_t* sig = reinterpret_cast<const uint8_t*>(signature_bytes.data());
+  const uint8_t* pub_key =
+      reinterpret_cast<const uint8_t*>(public_key_bytes.data());
+
+  int result = ED25519_verify(message, message_len, sig, pub_key);
+  if (result != 1) {
+    LOG(ERROR) << "browseros: Ed25519 signature verification failed";
+    return false;
+  }
+
+  LOG(INFO) << "browseros: Ed25519 signature verified successfully";
+  return true;
+}
+
+// Extracts ZIP file to destination directory.
+// Returns empty string on success, error message on failure.
+std::string ExtractZipFile(const base::FilePath& zip_path,
+                           const base::FilePath& dest_dir) {
+  // Ensure destination directory exists
+  if (!base::CreateDirectory(dest_dir)) {
+    return "Failed to create destination directory: " + dest_dir.AsUTF8Unsafe();
+  }
+
+  // Use the zip::Unzip utility which handles the extraction properly
+  if (!zip::Unzip(zip_path, dest_dir)) {
+    return "Failed to extract ZIP file";
+  }
+
+  LOG(INFO) << "browseros: Extracted ZIP to " << dest_dir;
+  return "";  // Success
+}
+
+// Background task: verify signature + extract ZIP
+struct VerifyExtractResult {
+  bool success = false;
+  std::string error;
+};
+
+VerifyExtractResult DoVerifyAndExtract(const base::FilePath& zip_path,
+                                       const std::string& signature,
+                                       const base::FilePath& dest_dir) {
+  VerifyExtractResult result;
+
+  // Step 1: Verify signature
+  if (!VerifyEd25519Signature(zip_path, signature, kServerUpdatePublicKey)) {
+    result.error = "Signature verification failed";
+    base::DeleteFile(zip_path);
+    return result;
+  }
+
+  // Step 2: Clean stale destination if exists (handles interrupted updates)
+  if (base::PathExists(dest_dir)) {
+    LOG(WARNING) << "browseros: Cleaning stale version directory: " << dest_dir;
+    if (!base::DeletePathRecursively(dest_dir)) {
+      result.error = "Failed to clean stale version directory";
+      base::DeleteFile(zip_path);
+      return result;
+    }
+  }
+
+  // Step 3: Extract ZIP
+  std::string extract_error = ExtractZipFile(zip_path, dest_dir);
+  if (!extract_error.empty()) {
+    result.error = extract_error;
+    // Cleanup partial extraction
+    base::DeletePathRecursively(dest_dir);
+    base::DeleteFile(zip_path);
+    return result;
+  }
+
+  // Success - delete the ZIP file (we have extracted contents)
+  base::DeleteFile(zip_path);
+  result.success = true;
+  return result;
+}
+
+}  // namespace
+
+BrowserOSServerUpdater::BrowserOSServerUpdater(
+    browseros::BrowserOSServerManager* manager)
+    : manager_(manager),
+      version_store_(&manager->GetVersionStore()),
+      descriptor_(&browseros::GetManagedServerDescriptor()) {}
+
+BrowserOSServerUpdater::~BrowserOSServerUpdater() {
+  Stop();
+}
+
+void BrowserOSServerUpdater::Start() {
+  LOG(INFO) << "browseros: Starting server update polling";
+  update_check_timer_.Start(FROM_HERE, kUpdateCheckInterval, this,
+                            &BrowserOSServerUpdater::OnUpdateTimer);
+  CheckNow();
+}
+
+void BrowserOSServerUpdater::Stop() {
+  LOG(INFO) << "browseros: Stopping server updater";
+  update_check_timer_.Stop();
+  weak_factory_.InvalidateWeakPtrs();
+  appcast_loader_.reset();
+  download_loader_.reset();
+  ResetState();
+}
+
+bool BrowserOSServerUpdater::IsUpdateInProgress() const {
+  return update_in_progress_;
+}
+
+void BrowserOSServerUpdater::CheckNow() {
+  if (!version_store_->initialized()) {
+    LOG(INFO) << "browseros: Version caches not loaded yet, skipping check";
+    return;
+  }
+
+  if (update_in_progress_) {
+    LOG(INFO) << "browseros: Update check already in progress, skipping";
+    return;
+  }
+
+  FetchAppcast();
+}
+
+void BrowserOSServerUpdater::OnUpdateTimer() {
+  CheckNow();
+}
+
+void BrowserOSServerUpdater::FetchAppcast() {
+  state_ = State::kFetchingAppcast;
+  update_in_progress_ = true;
+
+  // Get appcast URL (allow override via command line, otherwise use the
+  // selected product's alpha/stable feed). The override switch is
+  // product-agnostic because only the selected sidecar's updater runs.
+  std::string appcast_url;
+  base::CommandLine* cmd = base::CommandLine::ForCurrentProcess();
+  if (cmd->HasSwitch(browseros::kServerAppcastUrl)) {
+    appcast_url = cmd->GetSwitchValueASCII(browseros::kServerAppcastUrl);
+    LOG(INFO) << "browseros: Using custom appcast URL: " << appcast_url;
+  } else if (base::FeatureList::IsEnabled(features::kBrowserOsAlphaFeatures)) {
+    appcast_url = std::string(descriptor_->updater.alpha_appcast_url);
+  } else {
+    appcast_url = std::string(descriptor_->updater.appcast_url);
+  }
+
+  GURL url(appcast_url);
+  if (!url.is_valid()) {
+    OnError("check", "Invalid appcast URL: " + appcast_url);
+    return;
+  }
+
+  LOG(INFO) << "browseros: Fetching appcast from " << url;
+
+  auto request = std::make_unique<network::ResourceRequest>();
+  request->url = url;
+  request->method = "GET";
+  request->credentials_mode = network::mojom::CredentialsMode::kOmit;
+
+  appcast_loader_ = network::SimpleURLLoader::Create(
+      std::move(request), GetAppcastTrafficAnnotation());
+  appcast_loader_->SetTimeoutDuration(kAppcastFetchTimeout);
+
+  auto* url_loader_factory = g_browser_process->system_network_context_manager()
+                                 ->GetURLLoaderFactory();
+
+  appcast_loader_->DownloadToString(
+      url_loader_factory,
+      base::BindOnce(&BrowserOSServerUpdater::OnAppcastFetched,
+                     weak_factory_.GetWeakPtr()),
+      kMaxAppcastSize);
+}
+
+void BrowserOSServerUpdater::OnAppcastFetched(
+    std::optional<std::string> response) {
+  if (!response.has_value()) {
+    int net_error = appcast_loader_->NetError();
+    OnError("check",
+            "Failed to fetch appcast: " + net::ErrorToString(net_error));
+    return;
+  }
+
+  LOG(INFO) << "browseros: Received appcast (" << response->size() << " bytes)";
+
+  // Parse the appcast
+  std::optional<AppcastItem> item =
+      BrowserOSAppcastParser::ParseLatestItem(*response);
+  if (!item) {
+    OnError("check", "Failed to parse appcast XML");
+    return;
+  }
+
+  LOG(INFO) << "browseros: Latest version in appcast: "
+            << item->version.GetString();
+
+  // Find enclosure for current platform
+  const AppcastEnclosure* enclosure = item->GetEnclosureForCurrentPlatform();
+  if (!enclosure) {
+    OnError("check", "No enclosure found for current platform");
+    return;
+  }
+
+  LOG(INFO) << "browseros: Found enclosure for current platform: "
+            << enclosure->url;
+
+  // Discovery asks whether another download is needed. The manager separately
+  // compares its running installation with the store's ready installation.
+  base::Version current = version_store_->GetBestAvailable().version;
+  LOG(INFO) << "browseros: Best available version: "
+            << (current.IsValid() ? current.GetString() : "(none)");
+
+  if (current.IsValid() && current >= item->version) {
+    LOG(INFO) << "browseros: Latest server already available locally";
+    ResetState();
+    return;
+  }
+
+  LOG(INFO) << "browseros: New version available: "
+            << item->version.GetString();
+  pending_item_ = *item;
+  pending_signature_ = enclosure->signature;
+  CheckVersionAlreadyDownloaded(*enclosure, item->version);
+}
+
+void BrowserOSServerUpdater::CheckVersionAlreadyDownloaded(
+    const AppcastEnclosure& enclosure,
+    const base::Version& version) {
+  base::FilePath version_dir = GetVersionDir(version);
+
+  version_store_->file_task_runner()->PostTaskAndReplyWithResult(
+      FROM_HERE, base::BindOnce(&base::PathExists, version_dir),
+      base::BindOnce(&BrowserOSServerUpdater::OnVersionExistsCheck,
+                     weak_factory_.GetWeakPtr(), enclosure, version));
+}
+
+void BrowserOSServerUpdater::OnVersionExistsCheck(
+    const AppcastEnclosure& enclosure,
+    const base::Version& version,
+    bool exists) {
+  if (exists) {
+    LOG(INFO) << "browseros: Version " << version.GetString()
+              << " already downloaded, skipping to test";
+    TestBinary(version);
+    return;
+  }
+
+  StartDownload(enclosure, version);
+}
+
+void BrowserOSServerUpdater::StartDownload(const AppcastEnclosure& enclosure,
+                                           const base::Version& version) {
+  state_ = State::kDownloading;
+
+  GURL url(enclosure.url);
+  if (!url.is_valid()) {
+    OnError("download", "Invalid download URL: " + enclosure.url);
+    return;
+  }
+
+  // Prepare pending update directory
+  base::FilePath pending_dir = GetPendingUpdateDir();
+
+  // Clean up any previous pending update on background thread, then download
+  version_store_->file_task_runner()->PostTaskAndReply(
+      FROM_HERE,
+      base::BindOnce(
+          [](base::FilePath dir) {
+            if (base::PathExists(dir)) {
+              base::DeletePathRecursively(dir);
+            }
+            base::CreateDirectory(dir);
+          },
+          pending_dir),
+      base::BindOnce(
+          [](base::WeakPtr<BrowserOSServerUpdater> self,
+             const AppcastEnclosure& enc, const base::Version& ver) {
+            if (!self) {
+              return;
+            }
+
+            GURL download_url(enc.url);
+            LOG(INFO) << "browseros: Downloading " << download_url;
+
+            auto request = std::make_unique<network::ResourceRequest>();
+            request->url = download_url;
+            request->method = "GET";
+            request->credentials_mode = network::mojom::CredentialsMode::kOmit;
+
+            self->download_loader_ = network::SimpleURLLoader::Create(
+                std::move(request), GetDownloadTrafficAnnotation());
+            self->download_loader_->SetTimeoutDuration(kDownloadTimeout);
+
+            // Add progress logging (visible with --vmodule=*browseros*=1)
+            self->download_loader_->SetOnDownloadProgressCallback(
+                base::BindRepeating([](uint64_t current) {
+                  LOG(INFO) << "browseros: Download progress: "
+                            << (current / 1024 / 1024) << " MB";
+                }));
+
+            base::FilePath download_path =
+                self->GetPendingUpdateDir().AppendASCII(kDownloadFileName);
+
+            auto* url_loader_factory =
+                g_browser_process->system_network_context_manager()
+                    ->GetURLLoaderFactory();
+
+            self->download_loader_->DownloadToFile(
+                url_loader_factory,
+                base::BindOnce(&BrowserOSServerUpdater::OnDownloadComplete,
+                               self, ver),
+                download_path);
+          },
+          weak_factory_.GetWeakPtr(), enclosure, version));
+}
+
+void BrowserOSServerUpdater::OnDownloadComplete(const base::Version& version,
+                                                base::FilePath zip_path) {
+  if (zip_path.empty()) {
+    int net_error = download_loader_->NetError();
+    OnError("download", "Download failed: " + net::ErrorToString(net_error));
+    return;
+  }
+
+  LOG(INFO) << "browseros: Download complete: " << zip_path;
+
+  // Now verify and extract
+  VerifyAndExtract(zip_path, pending_signature_, version);
+}
+
+void BrowserOSServerUpdater::VerifyAndExtract(const base::FilePath& zip_path,
+                                              const std::string& signature,
+                                              const base::Version& version) {
+  state_ = State::kVerifying;
+
+  base::FilePath dest_dir = GetVersionDir(version);
+
+  LOG(INFO) << "browseros: Verifying signature and extracting to " << dest_dir;
+
+  // Run verification and extraction on background thread
+  version_store_->file_task_runner()->PostTaskAndReplyWithResult(
+      FROM_HERE,
+      base::BindOnce(&DoVerifyAndExtract, zip_path, signature, dest_dir),
+      base::BindOnce(
+          [](base::WeakPtr<BrowserOSServerUpdater> self, base::Version version,
+             VerifyExtractResult result) {
+            if (!self) {
+              return;
+            }
+            self->OnVerifyAndExtractComplete(version, result.success,
+                                             result.error);
+          },
+          weak_factory_.GetWeakPtr(), version));
+}
+
+void BrowserOSServerUpdater::OnVerifyAndExtractComplete(
+    const base::Version& version,
+    bool success,
+    const std::string& error) {
+  if (!success) {
+    OnError("verify", error);
+    return;
+  }
+
+  LOG(INFO) << "browseros: Verification and extraction successful";
+
+  // Test the binary
+  TestBinary(version);
+}
+
+void BrowserOSServerUpdater::TestBinary(const base::Version& version) {
+  state_ = State::kTesting;
+  version_store_->PrepareVersion(
+      version, base::BindOnce(&BrowserOSServerUpdater::OnVersionPrepared,
+                              weak_factory_.GetWeakPtr(), version));
+}
+
+void BrowserOSServerUpdater::OnVersionPrepared(const base::Version& version,
+                                               bool success) {
+  if (!success) {
+    OnError("prepare", "Binary version validation or ready persistence failed");
+    return;
+  }
+
+  LOG(INFO) << "browseros: Server version " << version.GetString()
+            << " is ready for activation";
+  // Ready state belongs to the store and survives both busy deferrals and
+  // browser shutdown. Only transient network work ends here.
+  CleanupPendingUpdate();
+  ResetState();
+  manager_->MaybeActivateReadyVersion();
+}
+
+void BrowserOSServerUpdater::OnServerActivated() {
+  CleanupOldVersions();
+}
+
+void BrowserOSServerUpdater::InvalidateDownloadedVersion() {
+  manager_->InvalidateDownloadedServer();
+}
+
+base::FilePath BrowserOSServerUpdater::GetVersionsDir() const {
+  return version_store_->GetVersionsDir();
+}
+
+base::FilePath BrowserOSServerUpdater::GetVersionDir(
+    const base::Version& version) const {
+  return version_store_->GetVersionDir(version);
+}
+
+base::FilePath BrowserOSServerUpdater::GetPendingUpdateDir() const {
+  return version_store_->GetPendingUpdateDir();
+}
+
+base::FilePath BrowserOSServerUpdater::GetBestServerBinaryPath() {
+  return version_store_->GetBestAvailable().executable;
+}
+
+base::FilePath BrowserOSServerUpdater::GetBestServerResourcesPath() {
+  return version_store_->GetBestAvailable().resources;
+}
+
+void BrowserOSServerUpdater::CleanupPendingUpdate() {
+  base::FilePath pending_dir = GetPendingUpdateDir();
+  version_store_->file_task_runner()->PostTask(
+      FROM_HERE, base::BindOnce(
+                     [](base::FilePath dir) {
+                       if (base::PathExists(dir)) {
+                         base::DeletePathRecursively(dir);
+                       }
+                     },
+                     pending_dir));
+}
+
+void BrowserOSServerUpdater::CleanupOldVersions() {
+  base::FilePath versions_dir = GetVersionsDir();
+
+  version_store_->file_task_runner()->PostTask(
+      FROM_HERE,
+      base::BindOnce(
+          [](base::FilePath dir, int max_to_keep, base::Version running_version,
+             base::Version ready_version) {
+            if (!base::PathExists(dir)) {
+              return;
+            }
+
+            // Collect all version directories
+            std::vector<std::pair<base::Version, base::FilePath>> versions;
+            base::FileEnumerator enumerator(dir, false,
+                                            base::FileEnumerator::DIRECTORIES);
+            for (base::FilePath path = enumerator.Next(); !path.empty();
+                 path = enumerator.Next()) {
+              base::Version version(path.BaseName().AsUTF8Unsafe());
+              if (version.IsValid()) {
+                versions.emplace_back(version, path);
+              }
+            }
+
+            // Sort by version (newest first)
+            std::sort(versions.begin(), versions.end(),
+                      [](const auto& first, const auto& second) {
+                        return first.first > second.first;
+                      });
+
+            // Delete old versions beyond the keep limit
+            int deleted = 0;
+            for (size_t index = max_to_keep; index < versions.size(); ++index) {
+              const auto& [version, version_path] = versions[index];
+              if ((running_version.IsValid() && version == running_version) ||
+                  (ready_version.IsValid() && version == ready_version)) {
+                continue;
+              }
+              LOG(INFO) << "browseros: Cleaning up old version: "
+                        << version.GetString();
+              base::DeletePathRecursively(version_path);
+              deleted++;
+            }
+
+            if (deleted > 0) {
+              base::DictValue props;
+              props.Set("deleted_count", deleted);
+              browseros_metrics::BrowserOSMetrics::Log("server.ota.cleanup",
+                                                       std::move(props));
+            }
+          },
+          versions_dir, kMaxVersionsToKeep, manager_->GetRunningVersion(),
+          version_store_->GetBestAvailable().version));
+}
+
+void BrowserOSServerUpdater::OnError(const std::string& stage,
+                                     const std::string& error) {
+  LOG(ERROR) << "browseros: Update error at " << stage << ": " << error;
+
+  base::DictValue props;
+  props.Set("stage", stage);
+  props.Set("error", error);
+  if (pending_item_.version.IsValid()) {
+    props.Set("version", pending_item_.version.GetString());
+  }
+  browseros_metrics::BrowserOSMetrics::Log("server.ota.error",
+                                           std::move(props));
+
+  CleanupPendingUpdate();
+  ResetState();
+}
+
+void BrowserOSServerUpdater::ResetState() {
+  state_ = State::kIdle;
+  update_in_progress_ = false;
+  appcast_loader_.reset();
+  download_loader_.reset();
+  pending_item_ = AppcastItem();
+  pending_signature_.clear();
+}
+
+}  // namespace browseros_server
