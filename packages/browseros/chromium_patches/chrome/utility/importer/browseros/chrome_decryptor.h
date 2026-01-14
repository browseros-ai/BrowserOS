diff --git a/chrome/utility/importer/browseros/chrome_decryptor.h b/chrome/utility/importer/browseros/chrome_decryptor.h
new file mode 100644
index 0000000000000..3805c007ec30c
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_decryptor.h
@@ -0,0 +1,45 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome data decryption interface
+
+#ifndef CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_DECRYPTOR_H_
+#define CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_DECRYPTOR_H_
+
+#include <string>
+
+#include "base/files/file_path.h"
+#include "build/build_config.h"
+
+namespace browseros_importer {
+
+// Result of key extraction attempt
+enum class KeyExtractionResult {
+  kSuccess,
+  kKeychainAccessDenied,      // macOS: user denied Keychain access
+  kKeychainEntryNotFound,     // macOS: Chrome Safe Storage not found
+  kDpapiDecryptFailed,        // Windows: CryptUnprotectData failed
+  kLocalStateNotFound,        // Windows: Local State file missing
+  kLocalStateParseError,      // Windows: JSON parse failed
+  kChromeVersionUnsupported,  // Windows: Chrome 127+ detected (App-Bound)
+  kPlatformNotSupported,      // Linux or unknown platform
+  kUnknownError,
+};
+
+// Extract Chrome's encryption key from the system.
+// |profile_path| should be the Chrome profile directory (e.g., .../Default)
+// Returns the raw encryption key (16 bytes on macOS, 32 bytes on Windows)
+// or empty string on failure.
+// |result| receives the detailed status.
+std::string ExtractChromeKey(const base::FilePath& profile_path,
+                             KeyExtractionResult* result);
+
+// Decrypt a Chrome-encrypted value (password_value or encrypted_value).
+// |ciphertext| is the raw blob from the database (includes v10 prefix).
+// |key| is the encryption key from ExtractChromeKey().
+// Returns true on success and sets |plaintext|.
+bool DecryptChromeValue(const std::string& ciphertext,
+                        const std::string& key,
+                        std::string* plaintext);
+
+}  // namespace browseros_importer
+
+#endif  // CHROME_UTILITY_IMPORTER_BROWSEROS_CHROME_DECRYPTOR_H_
