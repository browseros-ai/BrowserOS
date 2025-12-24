diff --git a/chrome/browser/browseros/server/browseros_appcast_parser.h b/chrome/browser/browseros/server/browseros_appcast_parser.h
new file mode 100644
index 0000000000000..bd996e8cb06d2
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_appcast_parser.h
@@ -0,0 +1,88 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_APPCAST_PARSER_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_APPCAST_PARSER_H_
+
+#include <optional>
+#include <string>
+#include <vector>
+
+#include "base/time/time.h"
+#include "base/version.h"
+
+namespace browseros_server {
+
+// Represents a single enclosure (download) in an appcast item.
+// Each enclosure targets a specific OS/architecture combination.
+struct AppcastEnclosure {
+  AppcastEnclosure();
+  ~AppcastEnclosure();
+  AppcastEnclosure(const AppcastEnclosure&);
+  AppcastEnclosure& operator=(const AppcastEnclosure&);
+  AppcastEnclosure(AppcastEnclosure&&);
+  AppcastEnclosure& operator=(AppcastEnclosure&&);
+
+  std::string url;
+  std::string os;         // "macos", "linux", "windows"
+  std::string arch;       // "arm64", "x86_64"
+  std::string signature;  // Ed25519 signature (base64)
+  int64_t length = 0;
+
+  // Returns true if this enclosure matches the current platform and arch.
+  bool MatchesCurrentPlatform() const;
+};
+
+// Represents a single item (version) in an appcast feed.
+struct AppcastItem {
+  AppcastItem();
+  ~AppcastItem();
+  AppcastItem(const AppcastItem&);
+  AppcastItem& operator=(const AppcastItem&);
+  AppcastItem(AppcastItem&&);
+  AppcastItem& operator=(AppcastItem&&);
+
+  base::Version version;
+  base::Time pub_date;
+  std::vector<AppcastEnclosure> enclosures;
+
+  // Returns the enclosure matching the current platform, or nullptr if none.
+  const AppcastEnclosure* GetEnclosureForCurrentPlatform() const;
+};
+
+// Parses Sparkle-style appcast XML to extract version and download information.
+//
+// Expected XML format:
+// <rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
+//   <channel>
+//     <item>
+//       <sparkle:version>0.30.0</sparkle:version>
+//       <pubDate>Wed, 13 Nov 2025 17:30:00 -0700</pubDate>
+//       <enclosure
+//         url="https://..."
+//         sparkle:os="macos"
+//         sparkle:arch="arm64"
+//         sparkle:edSignature="base64..."
+//         length="12345678"
+//         type="application/zip"/>
+//     </item>
+//   </channel>
+// </rss>
+class BrowserOSAppcastParser {
+ public:
+  // Parses the given XML string and returns the latest (first) item.
+  // Returns std::nullopt if parsing fails or no valid items are found.
+  static std::optional<AppcastItem> ParseLatestItem(const std::string& xml);
+
+  // Parses all items from the appcast XML.
+  // Returns empty vector if parsing fails.
+  static std::vector<AppcastItem> ParseAllItems(const std::string& xml);
+
+ private:
+  BrowserOSAppcastParser() = delete;
+};
+
+}  // namespace browseros_server
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_BROWSEROS_APPCAST_PARSER_H_
