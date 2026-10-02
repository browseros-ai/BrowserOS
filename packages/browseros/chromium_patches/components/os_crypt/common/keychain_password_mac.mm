diff --git a/components/os_crypt/common/keychain_password_mac.mm b/components/os_crypt/common/keychain_password_mac.mm
index 212b93095214422112afd346e36cd2838891725b..6422141bbe9154f6c0f4922a43a122cff2bf4572 100644
--- a/components/os_crypt/common/keychain_password_mac.mm
+++ b/components/os_crypt/common/keychain_password_mac.mm
@@ -15,6 +15,7 @@
 #include "base/rand_util.h"
 #include "base/strings/string_view_util.h"
 #include "build/branding_buildflags.h"
+#include "components/os_crypt/common/browseros_product_buildflags.h"
 #include "crypto/apple/keychain_v2.h"
 #include "third_party/abseil-cpp/absl/cleanup/cleanup.h"
 
@@ -35,8 +36,13 @@
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
