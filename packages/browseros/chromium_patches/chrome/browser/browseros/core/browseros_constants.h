diff --git a/chrome/browser/browseros/core/browseros_constants.h b/chrome/browser/browseros/core/browseros_constants.h
new file mode 100644
index 0000000000000..db4aede1d2396
--- /dev/null
+++ b/chrome/browser/browseros/core/browseros_constants.h
@@ -0,0 +1,216 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_CORE_BROWSEROS_CONSTANTS_H_
+#define CHROME_BROWSER_BROWSEROS_CORE_BROWSEROS_CONSTANTS_H_
+
+#include <cstddef>
+#include <string>
+#include <vector>
+
+#include "base/command_line.h"
+#include "chrome/browser/browseros/core/browseros_switches.h"
+
+namespace browseros {
+
+// Check if URL overrides are disabled via command line flag
+inline bool IsURLOverridesDisabled() {
+  return base::CommandLine::ForCurrentProcess()->HasSwitch(kDisableUrlOverrides);
+}
+
+// Agent V2 Extension ID
+inline constexpr char kAgentV2ExtensionId[] =
+    "bflpfmnmnokmjhmgnolecpppdbdophmk";
+
+// BrowserOS extension config URLs
+inline constexpr char kBrowserOSConfigUrl[] =
+    "https://cdn.browseros.com/extensions/extensions.json";
+inline constexpr char kBrowserOSAlphaConfigUrl[] =
+    "https://cdn.browseros.com/extensions/extensions.alpha.json";
+
+// Bug Reporter Extension ID
+inline constexpr char kBugReporterExtensionId[] =
+    "adlpneommgkgeanpaekgoaolcpncohkf";
+
+// Controller Extension ID
+inline constexpr char kControllerExtensionId[] =
+    "nlnihljpboknmfagkikhkdblbedophja";
+
+// uBlock Origin Extension ID (Chrome Web Store)
+inline constexpr char kUBlockOriginExtensionId[] =
+    "cjpalhdlnbpafiamejdnhcphjbkeiagm";
+
+// BrowserOS CDN update manifest URL
+// Used for extensions installed from local .crx files that don't have
+// an update_url in their manifest
+inline constexpr char kBrowserOSUpdateUrl[] =
+    "https://cdn.browseros.com/extensions/update-manifest.xml";
+inline constexpr char kBrowserOSAlphaUpdateUrl[] =
+    "https://cdn.browseros.com/extensions/update-manifest.alpha.xml";
+
+// chrome://browseros host constant
+inline constexpr char kBrowserOSHost[] = "browseros";
+
+// URL route mapping for chrome://browseros/* virtual URLs
+struct BrowserOSURLRoute {
+  const char* virtual_path;    // Path in chrome://browseros/*, e.g., "/ai"
+  const char* extension_id;    // Extension that handles this route
+  const char* extension_page;  // Page within extension, e.g., "options.html"
+  const char* extension_hash;  // Hash/fragment without #, e.g., "ai" (empty if none)
+};
+
+inline constexpr BrowserOSURLRoute kBrowserOSURLRoutes[] = {
+    {"/settings", kAgentV2ExtensionId, "options.html", ""},
+    {"/mcp", kAgentV2ExtensionId, "options.html", "mcp"},
+    {"/onboarding", kAgentV2ExtensionId, "onboarding.html", ""},
+};
+
+inline constexpr size_t kBrowserOSURLRoutesCount =
+    sizeof(kBrowserOSURLRoutes) / sizeof(kBrowserOSURLRoutes[0]);
+
+// Find a route for a given virtual path (e.g., "/ai")
+// Returns nullptr if no matching route found
+inline const BrowserOSURLRoute* FindBrowserOSRoute(const std::string& path) {
+  for (const auto& route : kBrowserOSURLRoutes) {
+    if (path == route.virtual_path) {
+      return &route;
+    }
+  }
+  return nullptr;
+}
+
+// Get the extension URL for a chrome://browseros/* path
+// Returns empty string if no matching route or if URL overrides are disabled
+// Example: "/ai" -> "chrome-extension://bflp.../options.html#ai"
+inline std::string GetBrowserOSExtensionURL(const std::string& virtual_path) {
+  if (IsURLOverridesDisabled()) {
+    return std::string();
+  }
+  const BrowserOSURLRoute* route = FindBrowserOSRoute(virtual_path);
+  if (!route) {
+    return std::string();
+  }
+  std::string url = std::string("chrome-extension://") + route->extension_id +
+                    "/" + route->extension_page;
+  if (route->extension_hash[0] != '\0') {
+    url += "#";
+    url += route->extension_hash;
+  }
+  return url;
+}
+
+// Check if an extension URL matches a BrowserOS route
+// If matched, returns the virtual URL (chrome://browseros/...)
+// Returns empty string if not a BrowserOS extension URL
+// Parameters:
+//   extension_id: from url.host()
+//   extension_path: from url.path(), e.g., "/options.html"
+//   extension_ref: from url.ref(), e.g., "ai" or "/ai" (normalized internally)
+// Fallback: If no exact hash match, falls back to route with empty hash for same page
+inline std::string GetBrowserOSVirtualURL(const std::string& extension_id,
+                                          const std::string& extension_path,
+                                          const std::string& extension_ref) {
+  if (IsURLOverridesDisabled()) {
+    return std::string();
+  }
+
+  // Normalize ref - strip leading slash if present (handles both #ai and #/ai)
+  std::string normalized_ref = extension_ref;
+  if (!normalized_ref.empty() && normalized_ref[0] == '/') {
+    normalized_ref = normalized_ref.substr(1);
+  }
+
+  const BrowserOSURLRoute* fallback_route = nullptr;
+
+  for (const auto& route : kBrowserOSURLRoutes) {
+    if (extension_id != route.extension_id) {
+      continue;
+    }
+
+    // Compare path (handle leading slash)
+    std::string route_path = std::string("/") + route.extension_page;
+    if (extension_path != route_path && extension_path != route.extension_page) {
+      continue;
+    }
+
+    // Exact hash match - return immediately
+    if (normalized_ref == route.extension_hash) {
+      return std::string("chrome://") + kBrowserOSHost + route.virtual_path;
+    }
+
+    // Track fallback: route with empty hash for same page
+    if (route.extension_hash[0] == '\0') {
+      fallback_route = &route;
+    }
+  }
+
+  // No exact match - use fallback if available
+  if (fallback_route) {
+    return std::string("chrome://") + kBrowserOSHost + fallback_route->virtual_path;
+  }
+
+  return std::string();
+}
+
+struct BrowserOSExtensionInfo {
+  const char* id;
+  bool is_pinned;
+  bool is_labelled;
+};
+
+inline constexpr BrowserOSExtensionInfo kBrowserOSExtensions[] = {
+    {kAgentV2ExtensionId, false, false},
+    {kBugReporterExtensionId, true, false},
+    {kControllerExtensionId, false, false},
+    // ublock origin gets installed from chrome web store
+    {kUBlockOriginExtensionId, false, false},
+};
+
+inline constexpr size_t kBrowserOSExtensionsCount =
+    sizeof(kBrowserOSExtensions) / sizeof(kBrowserOSExtensions[0]);
+
+inline const BrowserOSExtensionInfo* FindBrowserOSExtensionInfo(
+    const std::string& extension_id) {
+  for (const auto& info : kBrowserOSExtensions) {
+    if (extension_id == info.id)
+      return &info;
+  }
+  return nullptr;
+}
+
+// Check if an extension is a BrowserOS extension
+inline bool IsBrowserOSExtension(const std::string& extension_id) {
+  return FindBrowserOSExtensionInfo(extension_id) != nullptr;
+}
+
+inline bool IsBrowserOSPinnedExtension(const std::string& extension_id) {
+  const BrowserOSExtensionInfo* info =
+      FindBrowserOSExtensionInfo(extension_id);
+  return info && info->is_pinned;
+}
+
+inline bool IsBrowserOSLabelledExtension(const std::string& extension_id) {
+  const BrowserOSExtensionInfo* info =
+      FindBrowserOSExtensionInfo(extension_id);
+  return info && info->is_labelled;
+}
+
+// Returns true if this extension uses the contextual (tab-specific) side panel
+// toggle behavior. Currently only Agent V2 uses this.
+inline bool UsesContextualSidePanelToggle(const std::string& extension_id) {
+  return extension_id == kAgentV2ExtensionId;
+}
+
+// Get all BrowserOS extension IDs
+inline std::vector<std::string> GetBrowserOSExtensionIds() {
+  std::vector<std::string> ids;
+  ids.reserve(kBrowserOSExtensionsCount);
+  for (const auto& info : kBrowserOSExtensions)
+    ids.push_back(info.id);
+  return ids;
+}
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_CORE_BROWSEROS_CONSTANTS_H_
