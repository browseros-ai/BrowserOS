diff --git a/chrome/browser/browseros/core/browseros_switches.h b/chrome/browser/browseros/core/browseros_switches.h
new file mode 100644
index 0000000000000..98317c9cd677f
--- /dev/null
+++ b/chrome/browser/browseros/core/browseros_switches.h
@@ -0,0 +1,32 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_CORE_BROWSEROS_SWITCHES_H_
+#define CHROME_BROWSER_BROWSEROS_CORE_BROWSEROS_SWITCHES_H_
+
+namespace browseros {
+
+inline constexpr char kDisableServer[] = "disable-browseros-server";
+inline constexpr char kDisableServerUpdater[] =
+    "disable-browseros-server-updater";
+inline constexpr char kServerAppcastUrl[] = "browseros-server-appcast-url";
+inline constexpr char kServerResourcesDir[] = "browseros-server-resources-dir";
+inline constexpr char kCDPPort[] = "browseros-cdp-port";
+inline constexpr char kProxyPort[] = "browseros-proxy-port";
+inline constexpr char kServerPort[] = "browseros-server-port";
+inline constexpr char kDisableExtensions[] = "disable-browseros-extensions";
+inline constexpr char kExtensionsUrl[] = "browseros-extensions-url";
+inline constexpr char kDisableUrlOverrides[] =
+    "browseros-disable-url-overrides";
+inline constexpr char kSparkleUrl[] = "browseros-sparkle-url";
+inline constexpr char kSparkleForceCheck[] = "browseros-sparkle-force-check";
+inline constexpr char kSparkleDryRun[] = "sparkle-dry-run";
+inline constexpr char kSparkleSkipSignature[] = "sparkle-skip-signature";
+inline constexpr char kSparkleSpoofVersion[] = "sparkle-spoof-version";
+inline constexpr char kSparkleVerbose[] = "sparkle-verbose";
+inline constexpr char kBrowserOSProduct[] = "browseros-product";
+inline constexpr char kDockIcon[] = "browseros-dock-icon";
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_CORE_BROWSEROS_SWITCHES_H_
