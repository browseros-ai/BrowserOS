diff --git a/chrome/browser/browseros/server/browseros_server_constants.h b/chrome/browser/browseros/server/browseros_server_constants.h
new file mode 100644
index 0000000000000..d2c8229f8c805
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_server_constants.h
@@ -0,0 +1,52 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_CONSTANTS_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_CONSTANTS_H_
+
+#include "base/time/time.h"
+
+namespace browseros_server {
+
+// Appcast URLs for checking server updates
+inline constexpr char kDefaultAppcastUrl[] =
+    "https://cdn.browseros.com/appcast-server.xml";
+inline constexpr char kAlphaAppcastUrl[] =
+    "https://cdn.browseros.com/appcast-server.alpha.xml";
+
+// Interval between update checks
+inline constexpr base::TimeDelta kUpdateCheckInterval = base::Minutes(15);
+
+// Ed25519 public key for signature verification (base64-encoded)
+// This key verifies the authenticity of downloaded server binaries.
+inline constexpr char kServerUpdatePublicKey[] =
+    "LzQmcNuTsdB3/dsivo0eeN+jPfDoriRHAkkEJcfFs2A=";
+
+// Maximum number of old versions to keep in the versions directory
+inline constexpr int kMaxVersionsToKeep = 2;
+
+// Timeout for downloading update packages
+inline constexpr base::TimeDelta kDownloadTimeout = base::Minutes(10);
+
+// Timeout for fetching appcast XML
+inline constexpr base::TimeDelta kAppcastFetchTimeout = base::Seconds(30);
+
+// Timeout for fetching server status
+inline constexpr base::TimeDelta kStatusCheckTimeout = base::Seconds(5);
+
+// Maximum size of appcast XML (prevent DoS via huge responses)
+inline constexpr size_t kMaxAppcastSize = 512 * 1024;  // 512 KB
+
+// Maximum size of update package (prevent disk exhaustion)
+inline constexpr size_t kMaxUpdatePackageSize = 200 * 1024 * 1024;  // 200 MB
+
+// Directory and file names
+inline constexpr char kVersionsDirectoryName[] = "versions";
+inline constexpr char kCurrentVersionFileName[] = "current_version";
+inline constexpr char kPendingUpdateDirectoryName[] = "pending_update";
+inline constexpr char kDownloadFileName[] = "download.zip";
+
+}  // namespace browseros_server
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_SERVER_CONSTANTS_H_
