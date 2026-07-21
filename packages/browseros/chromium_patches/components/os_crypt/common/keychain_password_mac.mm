diff --git a/components/os_crypt/common/keychain_password_mac.mm b/components/os_crypt/common/keychain_password_mac.mm
index f240dc22ee391c98c8768af7141b7447b6507cc6..805f7e3f37a5b1d5cb76ee3f6f0b57b3e1026ea0 100644
--- a/components/os_crypt/common/keychain_password_mac.mm
+++ b/components/os_crypt/common/keychain_password_mac.mm
@@ -18,6 +18,7 @@
 #include "base/strings/string_view_util.h"
 #include "base/types/expected.h"
 #include "build/branding_buildflags.h"
+#include "components/os_crypt/common/browseros_product_buildflags.h"
 #include "crypto/apple/keychain_v2.h"
 #include "third_party/abseil-cpp/absl/cleanup/cleanup.h"
 
@@ -38,8 +39,13 @@ namespace {
 const char kDefaultServiceName[] = "Chrome Safe Storage";
 const char kDefaultAccountName[] = "Chrome";
 #else
-const char kDefaultServiceName[] = "Chromium Safe Storage";
-const char kDefaultAccountName[] = "Chromium";
+#if BUILDFLAG(BROWSEROS_PRODUCT_BROWSERCLAW)
+const char kDefaultServiceName[] = "BrowserClaw Safe Storage";
+const char kDefaultAccountName[] = "BrowserClaw";
+#else
+const char kDefaultServiceName[] = "BrowserOS Safe Storage";
+const char kDefaultAccountName[] = "BrowserOS";
+#endif
 #endif
 
 // These values are persisted to logs. Entries should not be renumbered and
