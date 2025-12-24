diff --git a/chrome/browser/browseros/server/browseros_appcast_parser.cc b/chrome/browser/browseros/server/browseros_appcast_parser.cc
new file mode 100644
index 0000000000000..dd30f75d414ec
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_appcast_parser.cc
@@ -0,0 +1,202 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_appcast_parser.h"
+
+#include <map>
+
+#include "base/logging.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/time/time.h"
+#include "build/build_config.h"
+#include "third_party/libxml/chromium/xml_reader.h"
+
+namespace browseros_server {
+
+// AppcastEnclosure
+AppcastEnclosure::AppcastEnclosure() = default;
+AppcastEnclosure::~AppcastEnclosure() = default;
+AppcastEnclosure::AppcastEnclosure(const AppcastEnclosure&) = default;
+AppcastEnclosure& AppcastEnclosure::operator=(const AppcastEnclosure&) = default;
+AppcastEnclosure::AppcastEnclosure(AppcastEnclosure&&) = default;
+AppcastEnclosure& AppcastEnclosure::operator=(AppcastEnclosure&&) = default;
+
+// AppcastItem
+AppcastItem::AppcastItem() = default;
+AppcastItem::~AppcastItem() = default;
+AppcastItem::AppcastItem(const AppcastItem&) = default;
+AppcastItem& AppcastItem::operator=(const AppcastItem&) = default;
+AppcastItem::AppcastItem(AppcastItem&&) = default;
+AppcastItem& AppcastItem::operator=(AppcastItem&&) = default;
+
+namespace {
+
+// Returns the current OS string used in appcast (matches Sparkle conventions).
+std::string GetCurrentOSString() {
+#if BUILDFLAG(IS_MAC)
+  return "macos";
+#elif BUILDFLAG(IS_LINUX)
+  return "linux";
+#elif BUILDFLAG(IS_WIN)
+  return "windows";
+#else
+  return "";
+#endif
+}
+
+// Returns the current architecture string used in appcast.
+std::string GetCurrentArchString() {
+#if defined(ARCH_CPU_ARM64)
+  return "arm64";
+#elif defined(ARCH_CPU_X86_64)
+  return "x86_64";
+#else
+  return "";
+#endif
+}
+
+// Parses an RFC 2822 date string (used in RSS pubDate).
+// Example: "Wed, 13 Nov 2025 17:30:00 -0700"
+base::Time ParseRFC2822Date(const std::string& date_str) {
+  base::Time time;
+  // Try parsing with timezone
+  if (base::Time::FromString(date_str.c_str(), &time)) {
+    return time;
+  }
+  // Return epoch if parsing fails
+  return base::Time();
+}
+
+// Parses a single <enclosure> element's attributes into an AppcastEnclosure.
+AppcastEnclosure ParseEnclosureFromAttributes(
+    const std::map<std::string, std::string>& attrs) {
+  AppcastEnclosure enclosure;
+
+  auto it = attrs.find("url");
+  if (it != attrs.end()) {
+    enclosure.url = it->second;
+  }
+
+  // Try both namespaced and non-namespaced attribute names
+  it = attrs.find("sparkle:os");
+  if (it != attrs.end()) {
+    enclosure.os = it->second;
+  }
+
+  it = attrs.find("sparkle:arch");
+  if (it != attrs.end()) {
+    enclosure.arch = it->second;
+  }
+
+  it = attrs.find("sparkle:edSignature");
+  if (it != attrs.end()) {
+    enclosure.signature = it->second;
+  }
+
+  it = attrs.find("length");
+  if (it != attrs.end()) {
+    base::StringToInt64(it->second, &enclosure.length);
+  }
+
+  return enclosure;
+}
+
+}  // namespace
+
+bool AppcastEnclosure::MatchesCurrentPlatform() const {
+  return os == GetCurrentOSString() && arch == GetCurrentArchString();
+}
+
+const AppcastEnclosure* AppcastItem::GetEnclosureForCurrentPlatform() const {
+  for (const auto& enclosure : enclosures) {
+    if (enclosure.MatchesCurrentPlatform()) {
+      return &enclosure;
+    }
+  }
+  return nullptr;
+}
+
+// static
+std::optional<AppcastItem> BrowserOSAppcastParser::ParseLatestItem(
+    const std::string& xml) {
+  std::vector<AppcastItem> items = ParseAllItems(xml);
+  if (items.empty()) {
+    return std::nullopt;
+  }
+  return std::move(items[0]);
+}
+
+// static
+std::vector<AppcastItem> BrowserOSAppcastParser::ParseAllItems(
+    const std::string& xml) {
+  std::vector<AppcastItem> items;
+
+  XmlReader reader;
+  if (!reader.Load(xml)) {
+    LOG(WARNING) << "browseros: Failed to load appcast XML";
+    return items;
+  }
+
+  // State machine for parsing
+  bool in_channel = false;
+  bool in_item = false;
+  AppcastItem current_item;
+  int item_depth = 0;
+
+  while (reader.Read()) {
+    std::string node_name = reader.NodeName();
+    int depth = reader.Depth();
+
+    if (reader.IsElement() || reader.IsEmptyElement()) {
+      // Opening or self-closing tag
+      if (node_name == "channel") {
+        in_channel = true;
+      } else if (node_name == "item" && in_channel) {
+        in_item = true;
+        item_depth = depth;
+        current_item = AppcastItem();
+      } else if (in_item) {
+        if (node_name == "version" || node_name == "sparkle:version") {
+          std::string version_str;
+          if (reader.ReadElementContent(&version_str)) {
+            current_item.version = base::Version(version_str);
+          }
+        } else if (node_name == "pubDate") {
+          std::string date_str;
+          if (reader.ReadElementContent(&date_str)) {
+            current_item.pub_date = ParseRFC2822Date(date_str);
+          }
+        } else if (node_name == "enclosure") {
+          std::map<std::string, std::string> attrs;
+          if (reader.GetAllNodeAttributes(&attrs)) {
+            AppcastEnclosure enclosure = ParseEnclosureFromAttributes(attrs);
+            if (!enclosure.url.empty()) {
+              current_item.enclosures.push_back(std::move(enclosure));
+            }
+          }
+        }
+      }
+    } else if (reader.IsClosingElement()) {
+      // Closing tag
+      if (node_name == "channel") {
+        in_channel = false;
+      } else if (node_name == "item" && in_item && depth == item_depth) {
+        in_item = false;
+        if (current_item.version.IsValid() &&
+            !current_item.enclosures.empty()) {
+          items.push_back(std::move(current_item));
+        } else {
+          LOG(WARNING) << "browseros: Skipping invalid appcast item (no valid "
+                       << "version or enclosures)";
+        }
+        current_item = AppcastItem();
+      }
+    }
+  }
+
+  VLOG(1) << "browseros: Parsed " << items.size() << " appcast items";
+  return items;
+}
+
+}  // namespace browseros_server
