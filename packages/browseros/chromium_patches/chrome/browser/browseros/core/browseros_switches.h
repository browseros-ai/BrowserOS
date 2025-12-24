diff --git a/chrome/browser/browseros/core/browseros_switches.h b/chrome/browser/browseros/core/browseros_switches.h
new file mode 100644
index 0000000000000..4e7932596534c
--- /dev/null
+++ b/chrome/browser/browseros/core/browseros_switches.h
@@ -0,0 +1,83 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_CORE_BROWSEROS_SWITCHES_H_
+#define CHROME_BROWSER_BROWSEROS_CORE_BROWSEROS_SWITCHES_H_
+
+namespace browseros {
+
+// =============================================================================
+// BrowserOS Command-Line Switches
+// =============================================================================
+// All BrowserOS-specific command-line flags are defined here.
+// Usage: --flag-name or --flag-name=value
+
+// === Server Switches ===
+
+// Disables the BrowserOS server entirely.
+inline constexpr char kDisableServer[] = "disable-browseros-server";
+
+// Disables the BrowserOS server OTA updater.
+inline constexpr char kDisableServerUpdater[] = "disable-browseros-server-updater";
+
+// Overrides the appcast URL for server updates (testing).
+inline constexpr char kServerAppcastUrl[] = "browseros-server-appcast-url";
+
+// Overrides the server resources directory path.
+inline constexpr char kServerResourcesDir[] = "browseros-server-resources-dir";
+
+// Overrides the CDP (Chrome DevTools Protocol) port.
+inline constexpr char kCDPPort[] = "browseros-cdp-port";
+
+// Overrides the MCP (Model Context Protocol) port.
+inline constexpr char kMCPPort[] = "browseros-mcp-port";
+
+// Overrides the Agent server port.
+inline constexpr char kAgentPort[] = "browseros-agent-port";
+
+// Overrides the Extension server port.
+inline constexpr char kExtensionPort[] = "browseros-extension-port";
+
+// === Extension Switches ===
+
+// Disables BrowserOS managed extensions.
+inline constexpr char kDisableExtensions[] = "disable-browseros-extensions";
+
+// Overrides the extensions config URL.
+inline constexpr char kExtensionsUrl[] = "browseros-extensions-url";
+
+// === URL Override Switches ===
+
+// Disables chrome://browseros/* URL overrides.
+// Useful for debugging to see raw extension URLs.
+inline constexpr char kDisableUrlOverrides[] = "browseros-disable-url-overrides";
+
+// === Sparkle Switches (macOS Browser Updates) ===
+
+// Overrides the Sparkle appcast URL for browser updates.
+inline constexpr char kSparkleUrl[] = "browseros-sparkle-url";
+
+// Forces an immediate Sparkle update check.
+inline constexpr char kSparkleForceCheck[] = "browseros-sparkle-force-check";
+
+// Runs Sparkle in dry-run mode (no actual updates).
+inline constexpr char kSparkleDryRun[] = "sparkle-dry-run";
+
+// Skips Sparkle signature verification (testing only).
+inline constexpr char kSparkleSkipSignature[] = "sparkle-skip-signature";
+
+// Spoofs the current version for Sparkle (testing).
+inline constexpr char kSparkleSpoofVersion[] = "sparkle-spoof-version";
+
+// Enables verbose Sparkle logging.
+inline constexpr char kSparkleVerbose[] = "sparkle-verbose";
+
+// === Misc Switches ===
+
+// Indicates this is the first run of BrowserOS.
+inline constexpr char kFirstRun[] = "browseros-first-run";
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_CORE_BROWSEROS_SWITCHES_H_
