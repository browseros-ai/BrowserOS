diff --git a/chrome/utility/importer/browseros/chrome_decryptor.cc b/chrome/utility/importer/browseros/chrome_decryptor.cc
new file mode 100644
index 0000000000000..cf7ccdf4336f7
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_decryptor.cc
@@ -0,0 +1,31 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome decryption - Linux stub (deferred implementation)
+
+#include "chrome/utility/importer/browseros/chrome_decryptor.h"
+
+#include "base/logging.h"
+#include "build/build_config.h"
+
+namespace browseros_importer {
+
+#if BUILDFLAG(IS_LINUX) || BUILDFLAG(IS_CHROMEOS)
+
+std::string ExtractChromeKey(const base::FilePath& profile_path,
+                             KeyExtractionResult* result) {
+  LOG(INFO) << "browseros: Linux key extraction not yet implemented";
+  if (result) {
+    *result = KeyExtractionResult::kPlatformNotSupported;
+  }
+  return std::string();
+}
+
+bool DecryptChromeValue(const std::string& ciphertext,
+                        const std::string& key,
+                        std::string* plaintext) {
+  LOG(INFO) << "browseros: Linux decryption not yet implemented";
+  return false;
+}
+
+#endif  // BUILDFLAG(IS_LINUX) || BUILDFLAG(IS_CHROMEOS)
+
+}  // namespace browseros_importer
