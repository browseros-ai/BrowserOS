diff --git a/chrome/utility/importer/browseros/chrome_extensions_importer.cc b/chrome/utility/importer/browseros/chrome_extensions_importer.cc
new file mode 100644
index 0000000000000..521ceadec9fc9
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_extensions_importer.cc
@@ -0,0 +1,98 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome extensions importer implementation
+
+#include "chrome/utility/importer/browseros/chrome_extensions_importer.h"
+
+#include "base/files/file_util.h"
+#include "base/json/json_reader.h"
+#include "base/logging.h"
+
+namespace browseros_importer {
+
+namespace {
+
+constexpr char kPreferencesFilename[] = "Preferences";
+constexpr char kSecurePreferencesFilename[] = "Secure Preferences";
+
+std::vector<std::string> GetExtensionsFromPreferencesFile(
+    const base::FilePath& preferences_path) {
+  std::vector<std::string> extension_ids;
+
+  std::string preferences_content;
+  if (!base::ReadFileToString(preferences_path, &preferences_content)) {
+    LOG(WARNING) << "browseros: Failed to read "
+                 << preferences_path.BaseName().value();
+    return extension_ids;
+  }
+
+  std::optional<base::Value::Dict> preferences = base::JSONReader::ReadDict(
+      preferences_content, base::JSON_PARSE_CHROMIUM_EXTENSIONS);
+  if (!preferences) {
+    LOG(WARNING) << "browseros: Failed to parse JSON from "
+                 << preferences_path.BaseName().value();
+    return extension_ids;
+  }
+
+  const base::Value::Dict* extensions_dict =
+      preferences->FindDictByDottedPath("extensions.settings");
+  if (!extensions_dict) {
+    return extension_ids;
+  }
+
+  for (const auto [key, value] : *extensions_dict) {
+    if (!value.is_dict()) {
+      continue;
+    }
+
+    const base::Value::Dict& dict = value.GetDict();
+
+    // Skip default-installed extensions
+    if (dict.FindBool("was_installed_by_default").value_or(false)) {
+      continue;
+    }
+
+    // Only import extensions from Chrome Web Store
+    if (!dict.FindBool("from_webstore").value_or(false)) {
+      continue;
+    }
+
+    extension_ids.push_back(key);
+  }
+
+  return extension_ids;
+}
+
+}  // namespace
+
+std::vector<std::string> ImportChromeExtensions(
+    const base::FilePath& profile_path) {
+  std::vector<std::string> extension_ids;
+
+  base::FilePath preferences_path =
+      profile_path.AppendASCII(kPreferencesFilename);
+  base::FilePath secure_preferences_path =
+      profile_path.AppendASCII(kSecurePreferencesFilename);
+
+  if (!base::PathExists(preferences_path) &&
+      !base::PathExists(secure_preferences_path)) {
+    LOG(WARNING) << "browseros: No preferences files found";
+    return extension_ids;
+  }
+
+  // Secure Preferences takes priority
+  if (base::PathExists(secure_preferences_path)) {
+    extension_ids = GetExtensionsFromPreferencesFile(secure_preferences_path);
+  }
+
+  // Also check regular Preferences for additional extensions
+  if (base::PathExists(preferences_path)) {
+    std::vector<std::string> pref_extension_ids =
+        GetExtensionsFromPreferencesFile(preferences_path);
+    extension_ids.insert(extension_ids.end(), pref_extension_ids.begin(),
+                         pref_extension_ids.end());
+  }
+
+  return extension_ids;
+}
+
+}  // namespace browseros_importer
