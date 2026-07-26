diff --git a/chrome/utility/importer/browseros/chrome_decryptor_mac.mm b/chrome/utility/importer/browseros/chrome_decryptor_mac.mm
new file mode 100644
index 0000000000000..762582371aee4
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_decryptor_mac.mm
@@ -0,0 +1,194 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome decryption - macOS implementation
+// Uses Keychain for key retrieval, PBKDF2 for key derivation, AES-128-CBC for decryption
+
+#include "chrome/utility/importer/browseros/chrome_decryptor.h"
+
+#include <Security/Security.h>
+
+#include "base/containers/span.h"
+#include "base/logging.h"
+#include "base/strings/string_util.h"
+#include "build/build_config.h"
+#include "crypto/apple/keychain_v2.h"
+#include "third_party/boringssl/src/include/openssl/evp.h"
+
+#if BUILDFLAG(IS_MAC)
+
+namespace browseros_importer {
+
+namespace {
+
+// Chrome's encryption constants (matching os_crypt_mac.mm)
+constexpr char kSalt[] = "saltysalt";
+constexpr size_t kSaltLength = 9;  // strlen("saltysalt")
+constexpr int kPbkdf2Iterations = 1003;
+constexpr size_t kDerivedKeyLength = 16;  // AES-128
+constexpr size_t kIvLength = 16;
+constexpr char kEncryptionVersionPrefix[] = "v10";
+constexpr size_t kEncryptionVersionPrefixLength = 3;
+// Windows-only App-Bound marker; guarded against defensively (see below).
+constexpr char kAppBoundVersionPrefix[] = "v20";
+
+// IV is 16 space characters
+constexpr uint8_t kIv[kIvLength] = {
+    ' ', ' ', ' ', ' ', ' ', ' ', ' ', ' ',
+    ' ', ' ', ' ', ' ', ' ', ' ', ' ', ' '
+};
+
+// Chrome's Keychain service and account names
+constexpr char kChromeKeychainService[] = "Chrome Safe Storage";
+constexpr char kChromeKeychainAccount[] = "Chrome";
+
+// Retrieve Chrome's password from macOS Keychain using the modern API
+bool GetChromePasswordFromKeychain(std::string* password) {
+  crypto::apple::KeychainV2& keychain =
+      crypto::apple::KeychainV2::GetInstance();
+
+  auto result = keychain.FindGenericPassword(kChromeKeychainService,
+                                             kChromeKeychainAccount);
+
+  if (!result.has_value()) {
+    OSStatus error = result.error();
+    if (error == errSecItemNotFound) {
+      LOG(WARNING) << "browseros: Chrome Safe Storage not found in Keychain";
+    } else if (error == errSecAuthFailed) {
+      LOG(WARNING) << "browseros: Keychain access denied";
+    } else {
+      LOG(WARNING) << "browseros: Keychain error: " << error;
+    }
+    return false;
+  }
+
+  const std::vector<uint8_t>& password_bytes = result.value();
+  password->assign(reinterpret_cast<const char*>(password_bytes.data()),
+                   password_bytes.size());
+  return true;
+}
+
+// Derive encryption key using PBKDF2-HMAC-SHA1
+bool DeriveKeyFromPassword(const std::string& password,
+                           std::vector<uint8_t>* derived_key) {
+  derived_key->resize(kDerivedKeyLength);
+
+  int result = PKCS5_PBKDF2_HMAC_SHA1(
+      password.data(),
+      password.length(),
+      reinterpret_cast<const uint8_t*>(kSalt),
+      kSaltLength,
+      kPbkdf2Iterations,
+      kDerivedKeyLength,
+      derived_key->data());
+
+  return result == 1;
+}
+
+// Decrypt AES-128-CBC encrypted data
+bool DecryptAesCbc(const std::vector<uint8_t>& key,
+                   const uint8_t* ciphertext,
+                   size_t ciphertext_length,
+                   std::string* plaintext) {
+  if (key.size() != kDerivedKeyLength) {
+    LOG(WARNING) << "browseros: Invalid key size";
+    return false;
+  }
+
+  // Initialize cipher context
+  bssl::ScopedEVP_CIPHER_CTX ctx;
+  if (!EVP_DecryptInit_ex(ctx.get(), EVP_aes_128_cbc(), nullptr,
+                          key.data(), kIv)) {
+    LOG(WARNING) << "browseros: EVP_DecryptInit_ex failed";
+    return false;
+  }
+
+  // Allocate output buffer (plaintext is at most ciphertext length)
+  std::vector<uint8_t> output(ciphertext_length + EVP_MAX_BLOCK_LENGTH);
+  int output_length = 0;
+
+  if (!EVP_DecryptUpdate(ctx.get(), output.data(), &output_length,
+                         ciphertext, ciphertext_length)) {
+    LOG(WARNING) << "browseros: EVP_DecryptUpdate failed";
+    return false;
+  }
+
+  int final_length = 0;
+  auto output_span = base::span(output);
+  if (!EVP_DecryptFinal_ex(ctx.get(),
+                           output_span.subspan(static_cast<size_t>(output_length)).data(),
+                           &final_length)) {
+    LOG(WARNING) << "browseros: EVP_DecryptFinal_ex failed - possible padding error";
+    return false;
+  }
+
+  plaintext->assign(reinterpret_cast<char*>(output.data()),
+                    output_length + final_length);
+  return true;
+}
+
+}  // namespace
+
+std::string ExtractChromeKey(const base::FilePath& profile_path,
+                             KeyExtractionResult* result) {
+  // Get Chrome's password from Keychain
+  std::string keychain_password;
+  if (!GetChromePasswordFromKeychain(&keychain_password)) {
+    if (result) {
+      *result = KeyExtractionResult::kKeychainEntryNotFound;
+    }
+    return std::string();
+  }
+
+  // Derive the encryption key using PBKDF2
+  std::vector<uint8_t> derived_key;
+  if (!DeriveKeyFromPassword(keychain_password, &derived_key)) {
+    LOG(WARNING) << "browseros: PBKDF2 key derivation failed";
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
+  return std::string(reinterpret_cast<char*>(derived_key.data()),
+                     derived_key.size());
+}
+
+DecryptResult DecryptChromeValue(const std::string& ciphertext,
+                                 const std::string& key,
+                                 std::string* plaintext) {
+  if (ciphertext.empty()) {
+    return DecryptResult::kEmpty;
+  }
+
+  // App-Bound Encryption is Windows-only, but guard defensively so a v20 blob
+  // is skipped rather than mistaken for plaintext.
+  if (base::StartsWith(ciphertext, kAppBoundVersionPrefix)) {
+    return DecryptResult::kAppBoundUnsupported;
+  }
+
+  // v10 values: AES-128-CBC under the Keychain-derived key.
+  if (base::StartsWith(ciphertext, kEncryptionVersionPrefix)) {
+    auto encrypted_span =
+        base::as_byte_span(ciphertext).subspan(kEncryptionVersionPrefixLength);
+    if (encrypted_span.empty()) {
+      LOG(WARNING) << "browseros: Empty ciphertext after v10 prefix";
+      return DecryptResult::kError;
+    }
+    std::vector<uint8_t> key_bytes(key.begin(), key.end());
+    return DecryptAesCbc(key_bytes, encrypted_span.data(),
+                         encrypted_span.size(), plaintext)
+               ? DecryptResult::kSuccess
+               : DecryptResult::kError;
+  }
+
+  // Unprefixed values are genuinely unencrypted on macOS.
+  *plaintext = ciphertext;
+  return DecryptResult::kSuccess;
+}
+
+}  // namespace browseros_importer
+
+#endif  // BUILDFLAG(IS_MAC)
