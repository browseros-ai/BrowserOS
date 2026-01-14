diff --git a/chrome/utility/importer/browseros/chrome_decryptor_mac.mm b/chrome/utility/importer/browseros/chrome_decryptor_mac.mm
new file mode 100644
index 0000000000000..c71a63015ff1d
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_decryptor_mac.mm
@@ -0,0 +1,188 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome decryption - macOS implementation
+// Uses Keychain for key retrieval, PBKDF2 for key derivation, AES-128-CBC for decryption
+
+#include "chrome/utility/importer/browseros/chrome_decryptor.h"
+
+#include <Security/Security.h>
+
+#include "base/logging.h"
+#include "base/strings/string_util.h"
+#include "build/build_config.h"
+#include "crypto/apple/keychain.h"
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
+  std::unique_ptr<crypto::apple::Keychain> keychain =
+      crypto::apple::Keychain::DefaultKeychain();
+
+  auto result = keychain->FindGenericPassword(kChromeKeychainService,
+                                               kChromeKeychainAccount);
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
+  if (!EVP_DecryptFinal_ex(ctx.get(), output.data() + output_length,
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
+  // Convert key string to vector
+  std::vector<uint8_t> key_bytes(key.begin(), key.end());
+
+  return DecryptAesCbc(key_bytes, encrypted_data, encrypted_length, plaintext);
+}
+
+}  // namespace browseros_importer
+
+#endif  // BUILDFLAG(IS_MAC)
