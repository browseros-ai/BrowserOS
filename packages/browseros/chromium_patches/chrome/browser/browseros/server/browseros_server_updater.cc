diff --git a/chrome/browser/browseros/server/browseros_server_updater.cc b/chrome/browser/browseros/server/browseros_server_updater.cc
new file mode 100644
index 0000000000000..ced363ffbabc2
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_updater.cc
@@ -0,0 +1,1075 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_server_updater.h"
+
+#include "base/base64.h"
+#include "base/command_line.h"
+#include "base/feature_list.h"
+#include "base/files/file_enumerator.h"
+#include "base/files/file_util.h"
+#include "base/json/json_reader.h"
+#include "base/logging.h"
+#include "base/path_service.h"
+#include "base/process/launch.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/strings/string_util.h"
+#include "base/task/thread_pool.h"
+#include "chrome/browser/browser_features.h"
+#include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_switches.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics.h"
+#include "chrome/browser/browseros/server/browseros_server_constants.h"
+#include "chrome/browser/browseros/server/browseros_server_manager.h"
+#include "chrome/browser/browseros/server/browseros_server_prefs.h"
+#include "chrome/browser/net/system_network_context_manager.h"
+#include "chrome/common/chrome_paths.h"
+#include "components/prefs/pref_service.h"
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
+net::NetworkTrafficAnnotationTag GetStatusTrafficAnnotation() {
+  return net::DefineNetworkTrafficAnnotation("browseros_server_status", R"(
+    semantics {
+      sender: "BrowserOS Server Updater"
+      description:
+        "Checks if the local BrowserOS server is ready for hot-swap update."
+      trigger: "When a new version is downloaded and ready to install."
+      data: "No user data sent, just an HTTP GET to localhost."
+      destination: LOCAL
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
+        "Essential for coordinating BrowserOS server updates."
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
+// Runs binary with --version and captures output.
+// Returns exit code and output via out parameters.
+void RunBinaryVersionCheck(const base::FilePath& binary_path,
+                           int* exit_code,
+                           std::string* output) {
+  base::CommandLine cmd(binary_path);
+  cmd.AppendSwitch("version");
+
+  std::string stdout_output;
+  std::string stderr_output;
+
+  base::LaunchOptions options;
+#if BUILDFLAG(IS_WIN)
+  options.start_hidden = true;
+#endif
+
+  // GetAppOutputWithExitCode runs the process and captures output
+  bool success = base::GetAppOutputAndError(cmd, &stdout_output);
+
+  if (success) {
+    *exit_code = 0;
+    *output = stdout_output;
+  } else {
+    *exit_code = 1;
+    *output = stdout_output.empty() ? "Process failed to run" : stdout_output;
+  }
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
+    : manager_(manager) {}
+
+BrowserOSServerUpdater::~BrowserOSServerUpdater() {
+  Stop();
+}
+
+void BrowserOSServerUpdater::Start() {
+  LOG(INFO) << "browseros: Starting server updater";
+
+  // Load both version caches async, then start checking
+  LoadVersionCachesAsync();
+
+  update_check_timer_.Start(FROM_HERE, kUpdateCheckInterval, this,
+                            &BrowserOSServerUpdater::OnUpdateTimer);
+}
+
+void BrowserOSServerUpdater::LoadVersionCachesAsync() {
+  // Load downloaded version from file
+  base::FilePath version_file =
+      GetExecutionDir().AppendASCII(kCurrentVersionFileName);
+
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_BLOCKING},
+      base::BindOnce(
+          [](base::FilePath path) -> std::string {
+            std::string content;
+            if (!base::ReadFileToString(path, &content)) {
+              return "";
+            }
+            std::string_view trimmed =
+                base::TrimWhitespaceASCII(content, base::TRIM_ALL);
+            return std::string(trimmed);
+          },
+          version_file),
+      base::BindOnce(&BrowserOSServerUpdater::OnDownloadedVersionLoaded,
+                     weak_factory_.GetWeakPtr()));
+
+  // Get bundled version by running binary with --version
+  base::FilePath bundled_binary = GetBundledBinaryPath();
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_BLOCKING},
+      base::BindOnce(
+          [](base::FilePath path) -> std::pair<int, std::string> {
+            int exit_code = 0;
+            std::string output;
+            RunBinaryVersionCheck(path, &exit_code, &output);
+            return {exit_code, output};
+          },
+          bundled_binary),
+      base::BindOnce(
+          [](base::WeakPtr<BrowserOSServerUpdater> self,
+             std::pair<int, std::string> result) {
+            if (self) {
+              self->OnBundledVersionLoaded(result.first, result.second);
+            }
+          },
+          weak_factory_.GetWeakPtr()));
+}
+
+void BrowserOSServerUpdater::OnDownloadedVersionLoaded(
+    const std::string& version_str) {
+  if (!version_str.empty()) {
+    cached_downloaded_version_ = base::Version(version_str);
+    LOG(INFO) << "browseros: Cached downloaded version: "
+              << cached_downloaded_version_.GetString();
+  }
+  downloaded_version_loaded_ = true;
+  CheckVersionCachesAndStart();
+}
+
+void BrowserOSServerUpdater::OnBundledVersionLoaded(int exit_code,
+                                                    const std::string& output) {
+  if (exit_code == 0 && !output.empty()) {
+    // Parse version from output (trim whitespace)
+    std::string_view trimmed =
+        base::TrimWhitespaceASCII(output, base::TRIM_ALL);
+    cached_bundled_version_ = base::Version(std::string(trimmed));
+    if (cached_bundled_version_.IsValid()) {
+      LOG(INFO) << "browseros: Cached bundled version: "
+                << cached_bundled_version_.GetString();
+    } else {
+      LOG(WARNING) << "browseros: Could not parse bundled version from: "
+                   << output;
+    }
+  } else {
+    LOG(WARNING) << "browseros: Failed to get bundled version (exit_code="
+                 << exit_code << ")";
+  }
+  bundled_version_loaded_ = true;
+  CheckVersionCachesAndStart();
+}
+
+void BrowserOSServerUpdater::CheckVersionCachesAndStart() {
+  if (!bundled_version_loaded_ || !downloaded_version_loaded_) {
+    return;  // Wait for both to complete
+  }
+
+  // Sync version pref with current best version
+  base::Version current = GetCurrentVersion();
+  if (current.IsValid()) {
+    PrefService* prefs = g_browser_process->local_state();
+    if (prefs) {
+      prefs->SetString(kServerVersion, current.GetString());
+    }
+  }
+
+  // Now trigger the first check
+  CheckNow();
+}
+
+void BrowserOSServerUpdater::Stop() {
+  LOG(INFO) << "browseros: Stopping server updater";
+  update_check_timer_.Stop();
+  appcast_loader_.reset();
+  download_loader_.reset();
+  status_loader_.reset();
+  ResetState();
+}
+
+bool BrowserOSServerUpdater::IsUpdateInProgress() const {
+  return update_in_progress_;
+}
+
+void BrowserOSServerUpdater::CheckNow() {
+  if (!bundled_version_loaded_ || !downloaded_version_loaded_) {
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
+  // Get appcast URL (allow override via command line, otherwise use
+  // alpha/stable)
+  std::string appcast_url;
+  base::CommandLine* cmd = base::CommandLine::ForCurrentProcess();
+  if (cmd->HasSwitch(browseros::kServerAppcastUrl)) {
+    appcast_url = cmd->GetSwitchValueASCII(browseros::kServerAppcastUrl);
+    LOG(INFO) << "browseros: Using custom appcast URL: " << appcast_url;
+  } else if (base::FeatureList::IsEnabled(features::kBrowserOsAlphaFeatures)) {
+    appcast_url = kAlphaAppcastUrl;
+  } else {
+    appcast_url = kDefaultAppcastUrl;
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
+    std::unique_ptr<std::string> response) {
+  if (!response) {
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
+  // Compare with current version
+  base::Version current = GetCurrentVersion();
+  LOG(INFO) << "browseros: Current version: "
+            << (current.IsValid() ? current.GetString() : "(none)");
+
+  if (current.IsValid() && current >= item->version) {
+    LOG(INFO) << "browseros: Already up to date";
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
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock()},
+      base::BindOnce(&base::PathExists, version_dir),
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
+  base::ThreadPool::PostTaskAndReply(
+      FROM_HERE, {base::MayBlock()},
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
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_BLOCKING},
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
+
+  base::FilePath binary_path = GetDownloadedBinaryPath(version);
+  LOG(INFO) << "browseros: Testing binary: " << binary_path;
+
+  // Run version check on background thread
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_BLOCKING},
+      base::BindOnce(
+          [](base::FilePath path) -> std::pair<int, std::string> {
+            int exit_code = 0;
+            std::string output;
+            RunBinaryVersionCheck(path, &exit_code, &output);
+            return {exit_code, output};
+          },
+          binary_path),
+      base::BindOnce(
+          [](base::WeakPtr<BrowserOSServerUpdater> self, base::Version version,
+             std::pair<int, std::string> result) {
+            if (!self) {
+              return;
+            }
+            self->OnBinaryTestComplete(version, result.first, result.second);
+          },
+          weak_factory_.GetWeakPtr(), version));
+}
+
+void BrowserOSServerUpdater::OnBinaryTestComplete(const base::Version& version,
+                                                  int exit_code,
+                                                  const std::string& output) {
+  if (exit_code != 0) {
+    LOG(ERROR) << "browseros: Binary test failed with exit code " << exit_code
+               << ": " << output;
+
+    // Delete the broken version
+    base::FilePath version_dir = GetVersionDir(version);
+    base::ThreadPool::PostTask(
+        FROM_HERE, {base::MayBlock()},
+        base::BindOnce(
+            [](base::FilePath dir) { base::DeletePathRecursively(dir); },
+            version_dir));
+
+    OnError("verify", "Binary --version check failed");
+    return;
+  }
+
+  LOG(INFO) << "browseros: Binary test passed: " << output;
+
+  // Check if server is ready for hot-swap
+  CheckServerStatus();
+}
+
+void BrowserOSServerUpdater::CheckServerStatus() {
+  GURL status_url("http://127.0.0.1:" +
+                  base::NumberToString(manager_->GetServerPort()) + "/status");
+
+  LOG(INFO) << "browseros: Checking server status at " << status_url;
+
+  auto request = std::make_unique<network::ResourceRequest>();
+  request->url = status_url;
+  request->method = "GET";
+  request->credentials_mode = network::mojom::CredentialsMode::kOmit;
+
+  status_loader_ = network::SimpleURLLoader::Create(
+      std::move(request), GetStatusTrafficAnnotation());
+  status_loader_->SetTimeoutDuration(kStatusCheckTimeout);
+
+  auto* url_loader_factory = g_browser_process->system_network_context_manager()
+                                 ->GetURLLoaderFactory();
+
+  status_loader_->DownloadToString(
+      url_loader_factory,
+      base::BindOnce(&BrowserOSServerUpdater::OnStatusFetched,
+                     weak_factory_.GetWeakPtr()),
+      4096);
+}
+
+void BrowserOSServerUpdater::OnStatusFetched(
+    std::unique_ptr<std::string> response) {
+  if (!response) {
+    int net_error = status_loader_->NetError();
+    LOG(WARNING) << "browseros: Failed to fetch server status: "
+                 << net::ErrorToString(net_error)
+                 << ", proceeding with update anyway";
+    OnServerStatusChecked(/*can_update=*/true);
+    return;
+  }
+
+  std::optional<base::Value> json = base::JSONReader::Read(*response);
+  if (!json || !json->is_dict()) {
+    LOG(WARNING)
+        << "browseros: Invalid status response, proceeding with update";
+    OnServerStatusChecked(/*can_update=*/true);
+    return;
+  }
+
+  const base::Value::Dict& dict = json->GetDict();
+  std::optional<bool> can_update = dict.FindBool("can_update");
+
+  if (!can_update.has_value()) {
+    LOG(WARNING) << "browseros: Status response missing can_update field";
+    OnServerStatusChecked(/*can_update=*/true);
+    return;
+  }
+
+  OnServerStatusChecked(can_update.value());
+}
+
+void BrowserOSServerUpdater::OnServerStatusChecked(bool can_update) {
+  if (!can_update) {
+    LOG(INFO) << "browseros: Server busy, will retry hot-swap at next check";
+
+    base::Value::Dict props;
+    props.Set("pending_version", pending_item_.version.GetString());
+    browseros_metrics::BrowserOSMetrics::Log("server.ota.busy",
+                                             std::move(props));
+
+    ResetState();
+    return;
+  }
+
+  PerformHotSwap(pending_item_.version);
+}
+
+void BrowserOSServerUpdater::PerformHotSwap(const base::Version& version) {
+  LOG(INFO) << "browseros: Performing hot-swap to version "
+            << version.GetString();
+
+  // Capture old version for metrics before updating
+  base::Version old_version = GetCurrentVersion();
+
+  // Update version file first (so restart uses new binary)
+  WriteCurrentVersionFile(version);
+
+  // Tell manager to restart the server with new binary
+  manager_->RestartServerForUpdate(
+      base::BindOnce(&BrowserOSServerUpdater::OnHotSwapComplete,
+                     weak_factory_.GetWeakPtr(), old_version, version));
+}
+
+void BrowserOSServerUpdater::OnHotSwapComplete(const base::Version& old_version,
+                                               const base::Version& new_version,
+                                               bool success) {
+  if (!success) {
+    LOG(ERROR) << "browseros: Hot-swap failed, reverting to bundled version";
+
+    // Clear downloaded version - this updates cache, pref (to bundled), and
+    // deletes the current_version file so next restart uses bundled
+    WriteCurrentVersionFile(base::Version());
+
+    OnError("hotswap", "Failed to restart server with new binary");
+    return;
+  }
+
+  LOG(INFO) << "browseros: Hot-swap successful! Now running version "
+            << new_version.GetString();
+
+  // Cleanup old versions and pending update
+  CleanupOldVersions();
+  CleanupPendingUpdate();
+
+  // Log success metric
+  base::Value::Dict props;
+  props.Set("old_version",
+            old_version.IsValid() ? old_version.GetString() : "none");
+  props.Set("new_version", new_version.GetString());
+  browseros_metrics::BrowserOSMetrics::Log("server.ota.success",
+                                           std::move(props));
+
+  ResetState();
+}
+
+base::Version BrowserOSServerUpdater::GetCurrentVersion() {
+  // Priority: downloaded version > bundled version
+  base::Version downloaded = GetLatestDownloadedVersion();
+  base::Version bundled = GetBundledVersion();
+
+  if (downloaded.IsValid() && (!bundled.IsValid() || downloaded > bundled)) {
+    return downloaded;
+  }
+  return bundled;
+}
+
+base::Version BrowserOSServerUpdater::GetBundledVersion() {
+  // Use cached version from running bundled binary --version
+  return cached_bundled_version_;
+}
+
+base::Version BrowserOSServerUpdater::GetLatestDownloadedVersion() {
+  // Use cached version to avoid blocking I/O on UI thread
+  return cached_downloaded_version_;
+}
+
+void BrowserOSServerUpdater::WriteCurrentVersionFile(
+    const base::Version& version) {
+  // Update cache immediately
+  cached_downloaded_version_ = version;
+
+  // Update version pref for observability
+  // When clearing (invalid version), show bundled version in pref
+  PrefService* prefs = g_browser_process->local_state();
+  if (prefs) {
+    std::string pref_version;
+    if (version.IsValid()) {
+      pref_version = version.GetString();
+    } else if (cached_bundled_version_.IsValid()) {
+      pref_version = cached_bundled_version_.GetString();
+    }
+    prefs->SetString(kServerVersion, pref_version);
+  }
+
+  base::FilePath version_file =
+      GetExecutionDir().AppendASCII(kCurrentVersionFileName);
+
+  if (version.IsValid()) {
+    base::ThreadPool::PostTask(
+        FROM_HERE, {base::MayBlock()},
+        base::BindOnce(
+            [](base::FilePath path, std::string content) {
+              base::WriteFile(path, content);
+            },
+            version_file, version.GetString()));
+  } else {
+    // Delete file when clearing downloaded version
+    base::ThreadPool::PostTask(
+        FROM_HERE, {base::MayBlock()},
+        base::BindOnce([](base::FilePath path) { base::DeleteFile(path); },
+                       version_file));
+  }
+}
+
+void BrowserOSServerUpdater::InvalidateDownloadedVersion() {
+  LOG(WARNING) << "browseros: Invalidating downloaded version, "
+               << "nuking versions directory";
+
+  // Clear cache, pref, and current_version file via shared logic
+  WriteCurrentVersionFile(base::Version());
+
+  // Additionally nuke all version directories
+  base::FilePath versions_dir = GetVersionsDir();
+  base::ThreadPool::PostTask(
+      FROM_HERE, {base::MayBlock()},
+      base::BindOnce(
+          [](base::FilePath versions_dir) {
+            if (base::PathExists(versions_dir)) {
+              if (!base::DeletePathRecursively(versions_dir)) {
+                LOG(ERROR) << "browseros: Failed to delete versions directory: "
+                           << versions_dir;
+              }
+            }
+          },
+          versions_dir));
+}
+
+base::FilePath BrowserOSServerUpdater::GetExecutionDir() const {
+  base::FilePath user_data_dir;
+  if (!base::PathService::Get(chrome::DIR_USER_DATA, &user_data_dir)) {
+    return base::FilePath();
+  }
+  return user_data_dir.Append(FILE_PATH_LITERAL(".browseros"));
+}
+
+base::FilePath BrowserOSServerUpdater::GetVersionsDir() const {
+  return GetExecutionDir().AppendASCII(kVersionsDirectoryName);
+}
+
+base::FilePath BrowserOSServerUpdater::GetVersionDir(
+    const base::Version& version) const {
+  return GetVersionsDir().AppendASCII(version.GetString());
+}
+
+base::FilePath BrowserOSServerUpdater::GetPendingUpdateDir() const {
+  return GetExecutionDir().AppendASCII(kPendingUpdateDirectoryName);
+}
+
+base::FilePath BrowserOSServerUpdater::GetBundledBinaryPath() const {
+  // Delegate to manager's existing logic
+  return manager_->GetBrowserOSServerExecutablePath();
+}
+
+base::FilePath BrowserOSServerUpdater::GetBundledResourcesPath() const {
+  return manager_->GetBrowserOSServerResourcesPath();
+}
+
+base::FilePath BrowserOSServerUpdater::GetDownloadedBinaryPath(
+    const base::Version& version) const {
+  base::FilePath binary = GetVersionDir(version)
+                              .Append(FILE_PATH_LITERAL("resources"))
+                              .Append(FILE_PATH_LITERAL("bin"))
+                              .Append(FILE_PATH_LITERAL("browseros_server"));
+#if BUILDFLAG(IS_WIN)
+  binary = binary.AddExtension(FILE_PATH_LITERAL(".exe"));
+#endif
+  return binary;
+}
+
+base::FilePath BrowserOSServerUpdater::GetDownloadedResourcesPath(
+    const base::Version& version) const {
+  return GetVersionDir(version).Append(FILE_PATH_LITERAL("resources"));
+}
+
+base::FilePath BrowserOSServerUpdater::GetBestServerBinaryPath() {
+  // Use cached versions to avoid blocking I/O
+  base::Version downloaded = cached_downloaded_version_;
+  base::Version bundled = cached_bundled_version_;
+
+  if (downloaded.IsValid() && (!bundled.IsValid() || downloaded > bundled)) {
+    base::FilePath path = GetDownloadedBinaryPath(downloaded);
+    // Note: We trust the cache - if binary doesn't exist, manager will handle
+    return path;
+  }
+
+  return GetBundledBinaryPath();
+}
+
+base::FilePath BrowserOSServerUpdater::GetBestServerResourcesPath() {
+  // Use cached versions to avoid blocking I/O
+  base::Version downloaded = cached_downloaded_version_;
+  base::Version bundled = cached_bundled_version_;
+
+  if (downloaded.IsValid() && (!bundled.IsValid() || downloaded > bundled)) {
+    return GetDownloadedResourcesPath(downloaded);
+  }
+
+  return GetBundledResourcesPath();
+}
+
+void BrowserOSServerUpdater::CleanupPendingUpdate() {
+  base::FilePath pending_dir = GetPendingUpdateDir();
+  base::ThreadPool::PostTask(FROM_HERE, {base::MayBlock()},
+                             base::BindOnce(
+                                 [](base::FilePath dir) {
+                                   if (base::PathExists(dir)) {
+                                     base::DeletePathRecursively(dir);
+                                   }
+                                 },
+                                 pending_dir));
+}
+
+void BrowserOSServerUpdater::CleanupOldVersions() {
+  base::FilePath versions_dir = GetVersionsDir();
+
+  base::ThreadPool::PostTask(
+      FROM_HERE, {base::MayBlock()},
+      base::BindOnce(
+          [](base::FilePath dir, int max_to_keep) {
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
+              base::Version v(path.BaseName().AsUTF8Unsafe());
+              if (v.IsValid()) {
+                versions.emplace_back(v, path);
+              }
+            }
+
+            // Sort by version (newest first)
+            std::sort(
+                versions.begin(), versions.end(),
+                [](const auto& a, const auto& b) { return a.first > b.first; });
+
+            // Delete old versions beyond the keep limit
+            int deleted = 0;
+            for (size_t i = max_to_keep; i < versions.size(); ++i) {
+              LOG(INFO) << "browseros: Cleaning up old version: "
+                        << versions[i].first.GetString();
+              base::DeletePathRecursively(versions[i].second);
+              deleted++;
+            }
+
+            if (deleted > 0) {
+              base::Value::Dict props;
+              props.Set("deleted_count", deleted);
+              browseros_metrics::BrowserOSMetrics::Log("server.ota.cleanup",
+                                                       std::move(props));
+            }
+          },
+          versions_dir, kMaxVersionsToKeep));
+}
+
+void BrowserOSServerUpdater::OnError(const std::string& stage,
+                                     const std::string& error) {
+  LOG(ERROR) << "browseros: Update error at " << stage << ": " << error;
+
+  base::Value::Dict props;
+  props.Set("stage", stage);
+  props.Set("error", error);
+  if (pending_item_.version.IsValid()) {
+    props.Set("version", pending_item_.version.GetString());
+  }
+  browseros_metrics::BrowserOSMetrics::Log("server.ota.error",
+                                           std::move(props));
+
+  // Clean version directory if we failed after extraction (test or hotswap
+  // stage)
+  if (pending_item_.version.IsValid() &&
+      (stage == "test" || stage == "hotswap")) {
+    base::FilePath version_dir = GetVersionDir(pending_item_.version);
+    base::ThreadPool::PostTask(
+        FROM_HERE, {base::MayBlock()},
+        base::BindOnce(
+            [](base::FilePath dir) {
+              if (base::PathExists(dir)) {
+                LOG(INFO) << "browseros: Cleaning up failed version: " << dir;
+                base::DeletePathRecursively(dir);
+              }
+            },
+            version_dir));
+  }
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
+  status_loader_.reset();
+  pending_item_ = AppcastItem();
+  pending_signature_.clear();
+}
+
+}  // namespace browseros_server
