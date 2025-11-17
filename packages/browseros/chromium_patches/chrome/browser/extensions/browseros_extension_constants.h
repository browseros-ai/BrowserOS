diff --git a/chrome/browser/extensions/blockbrowser_extension_constants.h b/chrome/browser/extensions/blockbrowser_extension_constants.h
new file mode 100644
index 0000000000000..e2ffd24bff8d5
--- /dev/null
+++ b/chrome/browser/extensions/blockbrowser_extension_constants.h
@@ -0,0 +1,118 @@
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
+// Bug Reporter Extension ID
+inline constexpr char kBugReporterExtensionId[] =
+    "adlpneommgkgeanpaekgoaolcpncohkf";
+
+// Controller Extension ID
+inline constexpr char kControllerExtensionId[] =
+    "nlnihljpboknmfagkikhkdblbedophja";
+
+// BlockBrowser CDN update manifest URL
+// Used for extensions installed from local .crx files that don't have
+// an update_url in their manifest
+inline constexpr char kBlockBrowserUpdateUrl[] =
+    "https://cdn.browseros.com/extensions/update-manifest.xml";
+
+// BlockBrowser extension config URL
+inline constexpr char kBlockBrowserConfigUrl[] =
+    "https://cdn.browseros.com/extensions/extensions.json";
+
+struct BlockBrowserExtensionInfo {
+  const char* id;
+  const char* display_name;
+  bool is_pinned;
+  bool is_labelled;
+};
+
+inline constexpr BlockBrowserExtensionInfo kBlockBrowserExtensions[] = {
+    {kAISidePanelExtensionId, "BlockBrowser", true, true},
+    {kBugReporterExtensionId, "BlockBrowser/bug-reporter", true, false},
+    {kControllerExtensionId, "BlockBrowser/controller", false, false},
+    {kAgentV2ExtensionId, "BlockBrowser", false, false},
+};
+
+// Allowlist of BlockBrowser extension IDs that are permitted to be installed.
+// Only extensions with these IDs will be loaded from the config.
+inline constexpr const char* kAllowedExtensions[] = {
+    kBlockBrowserExtensions[0].id,
+    kBlockBrowserExtensions[1].id,
+    kBlockBrowserExtensions[2].id,
+    kBlockBrowserExtensions[3].id,
+};
+
+inline constexpr size_t kBlockBrowserExtensionsCount =
+    sizeof(kBlockBrowserExtensions) / sizeof(kBlockBrowserExtensions[0]);
+
+inline const BlockBrowserExtensionInfo* FindBlockBrowserExtensionInfo(
+    const std::string& extension_id) {
+  for (const auto& info : kBlockBrowserExtensions) {
+    if (extension_id == info.id)
+      return &info;
+  }
+  return nullptr;
+}
+
+// Check if an extension is a BlockBrowser extension
+inline bool IsBlockBrowserExtension(const std::string& extension_id) {
+  return FindBlockBrowserExtensionInfo(extension_id) != nullptr;
+}
+
+inline bool IsBlockBrowserPinnedExtension(const std::string& extension_id) {
+  const BlockBrowserExtensionInfo* info =
+      FindBlockBrowserExtensionInfo(extension_id);
+  return info && info->is_pinned;
+}
+
+inline bool IsBlockBrowserLabelledExtension(const std::string& extension_id) {
+  const BlockBrowserExtensionInfo* info =
+      FindBlockBrowserExtensionInfo(extension_id);
+  return info && info->is_labelled;
+}
+
+// Get all BlockBrowser extension IDs
+inline std::vector<std::string> GetBlockBrowserExtensionIds() {
+  std::vector<std::string> ids;
+  ids.reserve(kBlockBrowserExtensionsCount);
+  for (const auto& info : kBlockBrowserExtensions)
+    ids.push_back(info.id);
+  return ids;
+}
+
+// Get display name for BlockBrowser extensions in omnibox
+// Returns the display name if extension_id is a BlockBrowser extension,
+// otherwise returns std::nullopt
+inline std::optional<std::string> GetExtensionDisplayName(
+    const std::string& extension_id) {
+  if (const BlockBrowserExtensionInfo* info =
+          FindBlockBrowserExtensionInfo(extension_id)) {
+    return info->display_name;
+  }
+  return std::nullopt;
+}
+
+}  // namespace browseros
+}  // namespace extensions
+
+#endif  // CHROME_BROWSER_EXTENSIONS_BROWSEROS_EXTENSION_CONSTANTS_H_
