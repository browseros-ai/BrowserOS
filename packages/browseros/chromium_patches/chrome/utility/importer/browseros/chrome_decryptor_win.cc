diff --git a/chrome/utility/importer/browseros/chrome_decryptor_win.cc b/chrome/utility/importer/browseros/chrome_decryptor_win.cc
new file mode 100644
index 0000000000000..5853c63c5e2d4
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_decryptor_win.cc
@@ -0,0 +1,246 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome decryption - Windows implementation
+// Uses DPAPI for key retrieval, AES-256-GCM for decryption
+
+#include "chrome/utility/importer/browseros/chrome_decryptor.h"
+
+#include <windows.h>
+#include <wincrypt.h>
+
+#include "base/base64.h"
+#include "base/files/file_util.h"
+#include "base/json/json_reader.h"
+#include "base/logging.h"
+#include "base/strings/string_util.h"
+#include "base/values.h"
+#include "build/build_config.h"
+#include "crypto/aead.h"
+
+#if BUILDFLAG(IS_WIN)
+
+namespace browseros_importer {
+
+namespace {
+
+// Chrome's encryption constants for Windows
+constexpr char kEncryptionVersionPrefix[] = "v10";
+constexpr size_t kEncryptionVersionPrefixLength = 3;
+constexpr char kDpapiPrefix[] = "DPAPI";
+constexpr size_t kDpapiPrefixLength = 5;
+constexpr size_t kNonceLength = 12;  // AES-GCM nonce
+constexpr size_t kAesKeyLength = 32;  // AES-256
+constexpr size_t kAuthTagLength = 16;  // GCM auth tag
+
+// Get the path to Chrome's Local State file
+base::FilePath GetLocalStatePath(const base::FilePath& profile_path) {
+  // profile_path is like .../User Data/Default
+  // Local State is at .../User Data/Local State
+  return profile_path.DirName().Append(FILE_PATH_LITERAL("Local State"));
+}
+
+// Extract and decode the encrypted key from Local State
+bool GetEncryptedKeyFromLocalState(const base::FilePath& local_state_path,
+                                   std::string* encrypted_key) {
+  std::string json_content;
+  if (!base::ReadFileToString(local_state_path, &json_content)) {
+    LOG(WARNING) << "browseros: Failed to read Local State file";
+    return false;
+  }
+
+  auto parsed = base::JSONReader::Read(json_content);
+  if (!parsed || !parsed->is_dict()) {
+    LOG(WARNING) << "browseros: Failed to parse Local State JSON";
+    return false;
+  }
+
+  const base::Value::Dict& dict = parsed->GetDict();
+  const base::Value::Dict* os_crypt = dict.FindDict("os_crypt");
+  if (!os_crypt) {
+    LOG(WARNING) << "browseros: No os_crypt section in Local State";
+    return false;
+  }
+
+  const std::string* encoded_key = os_crypt->FindString("encrypted_key");
+  if (!encoded_key || encoded_key->empty()) {
+    LOG(WARNING) << "browseros: No encrypted_key in os_crypt";
+    return false;
+  }
+
+  // Base64 decode the key
+  std::optional<std::vector<uint8_t>> decoded = base::Base64Decode(*encoded_key);
+  if (!decoded) {
+    LOG(WARNING) << "browseros: Failed to base64 decode encrypted_key";
+    return false;
+  }
+
+  encrypted_key->assign(decoded->begin(), decoded->end());
+  return true;
+}
+
+// Decrypt using Windows DPAPI
+bool DecryptWithDpapi(const std::string& encrypted_data,
+                      std::string* decrypted_data) {
+  if (encrypted_data.length() <= kDpapiPrefixLength) {
+    LOG(WARNING) << "browseros: Encrypted data too short";
+    return false;
+  }
+
+  // Check for "DPAPI" prefix
+  if (!base::StartsWith(encrypted_data, kDpapiPrefix)) {
+    LOG(WARNING) << "browseros: Missing DPAPI prefix";
+    return false;
+  }
+
+  // Strip "DPAPI" prefix
+  const uint8_t* data = reinterpret_cast<const uint8_t*>(encrypted_data.data()) +
+                        kDpapiPrefixLength;
+  size_t data_length = encrypted_data.length() - kDpapiPrefixLength;
+
+  DATA_BLOB input_blob;
+  input_blob.pbData = const_cast<BYTE*>(data);
+  input_blob.cbData = static_cast<DWORD>(data_length);
+
+  DATA_BLOB output_blob = {0};
+
+  if (!CryptUnprotectData(&input_blob, nullptr, nullptr, nullptr, nullptr,
+                          CRYPTPROTECT_UI_FORBIDDEN, &output_blob)) {
+    DWORD error = GetLastError();
+    LOG(WARNING) << "browseros: CryptUnprotectData failed with error: "
+                 << error;
+    return false;
+  }
+
+  if (output_blob.pbData && output_blob.cbData > 0) {
+    decrypted_data->assign(reinterpret_cast<char*>(output_blob.pbData),
+                           output_blob.cbData);
+    LocalFree(output_blob.pbData);
+    return true;
+  }
+
+  return false;
+}
+
+// Decrypt AES-256-GCM encrypted data
+bool DecryptAesGcm(const std::string& key,
+                   const uint8_t* ciphertext,
+                   size_t ciphertext_length,
+                   std::string* plaintext) {
+  if (key.size() != kAesKeyLength) {
+    LOG(WARNING) << "browseros: Invalid AES key size: " << key.size();
+    return false;
+  }
+
+  // Minimum: nonce (12) + auth tag (16) + at least 1 byte of data
+  if (ciphertext_length < kNonceLength + kAuthTagLength) {
+    LOG(WARNING) << "browseros: Ciphertext too short for AES-GCM";
+    return false;
+  }
+
+  // Extract nonce (first 12 bytes)
+  const uint8_t* nonce = ciphertext;
+
+  // Extract ciphertext + auth tag (remaining bytes)
+  const uint8_t* encrypted_data = ciphertext + kNonceLength;
+  size_t encrypted_length = ciphertext_length - kNonceLength;
+
+  // Use Chromium's crypto::Aead for decryption
+  crypto::Aead aead(crypto::Aead::AES_256_GCM);
+  aead.Init(base::as_byte_span(key));
+
+  std::optional<std::vector<uint8_t>> decrypted = aead.Open(
+      base::span<const uint8_t>(encrypted_data, encrypted_length),
+      base::span<const uint8_t>(nonce, kNonceLength),
+      base::span<const uint8_t>());  // empty additional data
+
+  if (!decrypted) {
+    LOG(WARNING) << "browseros: AES-GCM decryption failed";
+    return false;
+  }
+
+  plaintext->assign(decrypted->begin(), decrypted->end());
+  return true;
+}
+
+}  // namespace
+
+std::string ExtractChromeKey(const base::FilePath& profile_path,
+                             KeyExtractionResult* result) {
+  // Get path to Local State
+  base::FilePath local_state_path = GetLocalStatePath(profile_path);
+  if (!base::PathExists(local_state_path)) {
+    LOG(WARNING) << "browseros: Local State file not found at: "
+                 << local_state_path.value();
+    if (result) {
+      *result = KeyExtractionResult::kLocalStateNotFound;
+    }
+    return std::string();
+  }
+
+  // Extract encrypted key from JSON
+  std::string encrypted_key;
+  if (!GetEncryptedKeyFromLocalState(local_state_path, &encrypted_key)) {
+    if (result) {
+      *result = KeyExtractionResult::kLocalStateParseError;
+    }
+    return std::string();
+  }
+
+  // Decrypt with DPAPI
+  std::string decrypted_key;
+  if (!DecryptWithDpapi(encrypted_key, &decrypted_key)) {
+    if (result) {
+      *result = KeyExtractionResult::kDpapiDecryptFailed;
+    }
+    return std::string();
+  }
+
+  // Verify key length (should be 32 bytes for AES-256)
+  if (decrypted_key.size() != kAesKeyLength) {
+    LOG(WARNING) << "browseros: Unexpected key length: "
+                 << decrypted_key.size() << " (expected " << kAesKeyLength << ")";
+    if (result) {
+      *result = KeyExtractionResult::kUnknownError;
+    }
+    return std::string();
+  }
+
+  if (result) {
+    *result = KeyExtractionResult::kSuccess;
+  }
+
+  return decrypted_key;
+}
+
+bool DecryptChromeValue(const std::string& ciphertext,
+                        const std::string& key,
+                        std::string* plaintext) {
+  if (ciphertext.empty()) {
+    return false;
+  }
+
+  // Check for v10 prefix (Chrome's encryption marker)
+  if (ciphertext.length() < kEncryptionVersionPrefixLength ||
+      !base::StartsWith(ciphertext, kEncryptionVersionPrefix)) {
+    // Not encrypted with v10, might be plaintext or old format
+    LOG(INFO) << "browseros: Value doesn't have v10 prefix, may be unencrypted";
+    *plaintext = ciphertext;
+    return true;
+  }
+
+  // Extract the actual encrypted data (skip "v10" prefix)
+  const uint8_t* encrypted_data =
+      reinterpret_cast<const uint8_t*>(ciphertext.data()) +
+      kEncryptionVersionPrefixLength;
+  size_t encrypted_length = ciphertext.length() - kEncryptionVersionPrefixLength;
+
+  if (encrypted_length == 0) {
+    LOG(WARNING) << "browseros: Empty ciphertext after prefix";
+    return false;
+  }
+
+  return DecryptAesGcm(key, encrypted_data, encrypted_length, plaintext);
+}
+
+}  // namespace browseros_importer
+
+#endif  // BUILDFLAG(IS_WIN)
