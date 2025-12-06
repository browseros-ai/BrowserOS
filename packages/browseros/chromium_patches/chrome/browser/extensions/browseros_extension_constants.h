diff --git a/chrome/browser/extensions/browseros_extension_constants.h b/chrome/browser/extensions/browseros_extension_constants.h
new file mode 100644
index 0000000000000..fa424c0921d07
--- /dev/null
+++ b/chrome/browser/extensions/browseros_extension_constants.h
@@ -0,0 +1,126 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_EXTENSIONS_BROWSEROS_EXTENSION_CONSTANTS_H_
+#define CHROME_BROWSER_EXTENSIONS_BROWSEROS_EXTENSION_CONSTANTS_H_
+
+#include <cstddef>
+#include <optional>
+#include <string>
+#include <vector>
+
+namespace extensions {
+namespace browseros {
+
+// AI Agent Extension ID
+inline constexpr char kAISidePanelExtensionId[] =
+    "djhdjhlnljbjgejbndockeedocneiaei";
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
+// BrowserOS CDN update manifest URL
+// Used for extensions installed from local .crx files that don't have
+// an update_url in their manifest
+inline constexpr char kBrowserOSUpdateUrl[] =
+    "https://cdn.browseros.com/extensions/update-manifest.xml";
+
+struct BrowserOSExtensionInfo {
+  const char* id;
+  const char* display_name;
+  bool is_pinned;
+  bool is_labelled;
+};
+
+inline constexpr BrowserOSExtensionInfo kBrowserOSExtensions[] = {
+    {kAISidePanelExtensionId, "BrowserOS", true, true},
+    {kBugReporterExtensionId, "BrowserOS/bug-reporter", true, false},
+    {kControllerExtensionId, "BrowserOS/controller", false, false},
+    {kAgentV2ExtensionId, "BrowserOS", true, true},
+};
+
+// Allowlist of BrowserOS extension IDs that are permitted to be installed.
+// Only extensions with these IDs will be loaded from the config.
+inline constexpr const char* kAllowedExtensions[] = {
+    kBrowserOSExtensions[0].id,
+    kBrowserOSExtensions[1].id,
+    kBrowserOSExtensions[2].id,
+    kBrowserOSExtensions[3].id,
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
+// Get display name for BrowserOS extensions in omnibox
+// Returns the display name if extension_id is a BrowserOS extension,
+// otherwise returns std::nullopt
+inline std::optional<std::string> GetExtensionDisplayName(
+    const std::string& extension_id) {
+  if (const BrowserOSExtensionInfo* info =
+          FindBrowserOSExtensionInfo(extension_id)) {
+    return info->display_name;
+  }
+  return std::nullopt;
+}
+
+}  // namespace browseros
+}  // namespace extensions
+
+#endif  // CHROME_BROWSER_EXTENSIONS_BROWSEROS_EXTENSION_CONSTANTS_H_
