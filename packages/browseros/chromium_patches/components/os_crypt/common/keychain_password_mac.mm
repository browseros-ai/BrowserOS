diff --git a/components/os_crypt/common/keychain_password_mac.mm b/components/os_crypt/common/keychain_password_mac.mm
index caa0e420956a3..d60a67a8bacb7 100644
--- a/components/os_crypt/common/keychain_password_mac.mm
+++ b/components/os_crypt/common/keychain_password_mac.mm
@@ -35,8 +35,9 @@
 const char kDefaultServiceName[] = "Chrome Safe Storage";
 const char kDefaultAccountName[] = "Chrome";
 #else
-const char kDefaultServiceName[] = "Chromium Safe Storage";
-const char kDefaultAccountName[] = "Chromium";
+// BrowserOS: custom keychain service name
+const char kDefaultServiceName[] = "BrowserOS Safe Storage";
+const char kDefaultAccountName[] = "BrowserOS";
 #endif
 
 // These values are persisted to logs. Entries should not be renumbered and
