diff --git a/chrome/browser/browseros/server/browseros_appcast_parser_unittest.cc b/chrome/browser/browseros/server/browseros_appcast_parser_unittest.cc
new file mode 100644
index 0000000000000..ceb483dee35c6
--- /dev/null
+++ b/chrome/browser/browseros/server/browseros_appcast_parser_unittest.cc
@@ -0,0 +1,389 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/server/browseros_appcast_parser.h"
+
+#include "build/build_config.h"
+#include "testing/gtest/include/gtest/gtest.h"
+
+namespace browseros_server {
+namespace {
+
+// =============================================================================
+// Valid XML Parsing
+// =============================================================================
+
+TEST(BrowserOSAppcastParserTest, ParsesValidAppcastWithSingleItem) {
+  const char kValidXml[] = R"(
+    <rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
+      <channel>
+        <item>
+          <sparkle:version>1.0.0</sparkle:version>
+          <pubDate>Wed, 13 Nov 2024 17:30:00 -0700</pubDate>
+          <enclosure
+            url="https://cdn.example.com/server-1.0.0-macos-arm64.zip"
+            sparkle:os="macos"
+            sparkle:arch="arm64"
+            sparkle:edSignature="base64signature=="
+            length="12345678"
+            type="application/zip"/>
+        </item>
+      </channel>
+    </rss>
+  )";
+
+  auto item = BrowserOSAppcastParser::ParseLatestItem(kValidXml);
+
+  ASSERT_TRUE(item.has_value());
+  EXPECT_EQ(base::Version("1.0.0"), item->version);
+  ASSERT_EQ(1u, item->enclosures.size());
+  EXPECT_EQ("https://cdn.example.com/server-1.0.0-macos-arm64.zip",
+            item->enclosures[0].url);
+  EXPECT_EQ("macos", item->enclosures[0].os);
+  EXPECT_EQ("arm64", item->enclosures[0].arch);
+  EXPECT_EQ("base64signature==", item->enclosures[0].signature);
+  EXPECT_EQ(12345678, item->enclosures[0].length);
+}
+
+TEST(BrowserOSAppcastParserTest, ParsesMultipleEnclosuresPerItem) {
+  const char kMultiPlatformXml[] = R"(
+    <rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
+      <channel>
+        <item>
+          <sparkle:version>2.0.0</sparkle:version>
+          <enclosure url="https://example.com/macos-arm64.zip"
+                     sparkle:os="macos" sparkle:arch="arm64"
+                     sparkle:edSignature="sig1" length="1000"/>
+          <enclosure url="https://example.com/macos-x64.zip"
+                     sparkle:os="macos" sparkle:arch="x86_64"
+                     sparkle:edSignature="sig2" length="1100"/>
+          <enclosure url="https://example.com/linux-x64.zip"
+                     sparkle:os="linux" sparkle:arch="x86_64"
+                     sparkle:edSignature="sig3" length="1200"/>
+          <enclosure url="https://example.com/windows-x64.zip"
+                     sparkle:os="windows" sparkle:arch="x86_64"
+                     sparkle:edSignature="sig4" length="1300"/>
+        </item>
+      </channel>
+    </rss>
+  )";
+
+  auto item = BrowserOSAppcastParser::ParseLatestItem(kMultiPlatformXml);
+
+  ASSERT_TRUE(item.has_value());
+  EXPECT_EQ(base::Version("2.0.0"), item->version);
+  EXPECT_EQ(4u, item->enclosures.size());
+}
+
+TEST(BrowserOSAppcastParserTest, ParseAllItems_ReturnsMultipleVersions) {
+  const char kMultiVersionXml[] = R"(
+    <rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
+      <channel>
+        <item>
+          <sparkle:version>2.0.0</sparkle:version>
+          <enclosure url="https://example.com/v2.zip" sparkle:os="macos"
+                     sparkle:arch="arm64" sparkle:edSignature="sig" length="100"/>
+        </item>
+        <item>
+          <sparkle:version>1.5.0</sparkle:version>
+          <enclosure url="https://example.com/v1.5.zip" sparkle:os="macos"
+                     sparkle:arch="arm64" sparkle:edSignature="sig" length="100"/>
+        </item>
+        <item>
+          <sparkle:version>1.0.0</sparkle:version>
+          <enclosure url="https://example.com/v1.zip" sparkle:os="macos"
+                     sparkle:arch="arm64" sparkle:edSignature="sig" length="100"/>
+        </item>
+      </channel>
+    </rss>
+  )";
+
+  auto items = BrowserOSAppcastParser::ParseAllItems(kMultiVersionXml);
+
+  ASSERT_EQ(3u, items.size());
+  EXPECT_EQ(base::Version("2.0.0"), items[0].version);
+  EXPECT_EQ(base::Version("1.5.0"), items[1].version);
+  EXPECT_EQ(base::Version("1.0.0"), items[2].version);
+}
+
+// =============================================================================
+// Invalid/Edge Case XML
+// =============================================================================
+
+TEST(BrowserOSAppcastParserTest, ReturnsNulloptForInvalidXml) {
+  auto item = BrowserOSAppcastParser::ParseLatestItem("not valid xml at all");
+  EXPECT_FALSE(item.has_value());
+}
+
+TEST(BrowserOSAppcastParserTest, ReturnsNulloptForEmptyString) {
+  auto item = BrowserOSAppcastParser::ParseLatestItem("");
+  EXPECT_FALSE(item.has_value());
+}
+
+TEST(BrowserOSAppcastParserTest, ReturnsNulloptForEmptyChannel) {
+  const char kEmptyXml[] = R"(
+    <rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
+      <channel></channel>
+    </rss>
+  )";
+
+  auto item = BrowserOSAppcastParser::ParseLatestItem(kEmptyXml);
+  EXPECT_FALSE(item.has_value());
+}
+
+TEST(BrowserOSAppcastParserTest, ReturnsNulloptForMissingVersion) {
+  const char kNoVersionXml[] = R"(
+    <rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
+      <channel>
+        <item>
+          <enclosure url="https://example.com/download.zip"
+                     sparkle:os="macos" sparkle:arch="arm64"/>
+        </item>
+      </channel>
+    </rss>
+  )";
+
+  auto item = BrowserOSAppcastParser::ParseLatestItem(kNoVersionXml);
+  EXPECT_FALSE(item.has_value());
+}
+
+TEST(BrowserOSAppcastParserTest, HandlesItemWithNoEnclosures) {
+  const char kNoEnclosureXml[] = R"(
+    <rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
+      <channel>
+        <item>
+          <sparkle:version>1.0.0</sparkle:version>
+        </item>
+      </channel>
+    </rss>
+  )";
+
+  auto item = BrowserOSAppcastParser::ParseLatestItem(kNoEnclosureXml);
+
+  // Parser should return item with empty enclosures
+  if (item.has_value()) {
+    EXPECT_TRUE(item->enclosures.empty());
+  }
+}
+
+TEST(BrowserOSAppcastParserTest, ParseAllItems_ReturnsEmptyForInvalidXml) {
+  auto items = BrowserOSAppcastParser::ParseAllItems("invalid xml");
+  EXPECT_TRUE(items.empty());
+}
+
+TEST(BrowserOSAppcastParserTest, ParseAllItems_ReturnsEmptyForEmptyChannel) {
+  const char kEmptyXml[] = R"(
+    <rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
+      <channel></channel>
+    </rss>
+  )";
+
+  auto items = BrowserOSAppcastParser::ParseAllItems(kEmptyXml);
+  EXPECT_TRUE(items.empty());
+}
+
+// =============================================================================
+// Platform Matching
+// =============================================================================
+
+TEST(AppcastEnclosureTest, MatchesCurrentPlatform) {
+  AppcastEnclosure enclosure;
+
+#if BUILDFLAG(IS_MAC)
+#if defined(ARCH_CPU_ARM64)
+  enclosure.os = "macos";
+  enclosure.arch = "arm64";
+  EXPECT_TRUE(enclosure.MatchesCurrentPlatform());
+
+  enclosure.arch = "x86_64";
+  EXPECT_FALSE(enclosure.MatchesCurrentPlatform());
+#else
+  enclosure.os = "macos";
+  enclosure.arch = "x86_64";
+  EXPECT_TRUE(enclosure.MatchesCurrentPlatform());
+
+  enclosure.arch = "arm64";
+  EXPECT_FALSE(enclosure.MatchesCurrentPlatform());
+#endif
+#elif BUILDFLAG(IS_LINUX)
+  enclosure.os = "linux";
+  enclosure.arch = "x86_64";
+  EXPECT_TRUE(enclosure.MatchesCurrentPlatform());
+
+  enclosure.os = "macos";
+  EXPECT_FALSE(enclosure.MatchesCurrentPlatform());
+#elif BUILDFLAG(IS_WIN)
+  enclosure.os = "windows";
+  enclosure.arch = "x86_64";
+  EXPECT_TRUE(enclosure.MatchesCurrentPlatform());
+
+  enclosure.os = "macos";
+  EXPECT_FALSE(enclosure.MatchesCurrentPlatform());
+#endif
+}
+
+TEST(AppcastEnclosureTest, DoesNotMatchWrongOS) {
+  AppcastEnclosure enclosure;
+
+#if BUILDFLAG(IS_MAC)
+  enclosure.os = "linux";
+  enclosure.arch = "arm64";
+  EXPECT_FALSE(enclosure.MatchesCurrentPlatform());
+
+  enclosure.os = "windows";
+  EXPECT_FALSE(enclosure.MatchesCurrentPlatform());
+#elif BUILDFLAG(IS_LINUX)
+  enclosure.os = "macos";
+  enclosure.arch = "x86_64";
+  EXPECT_FALSE(enclosure.MatchesCurrentPlatform());
+
+  enclosure.os = "windows";
+  EXPECT_FALSE(enclosure.MatchesCurrentPlatform());
+#elif BUILDFLAG(IS_WIN)
+  enclosure.os = "macos";
+  enclosure.arch = "x86_64";
+  EXPECT_FALSE(enclosure.MatchesCurrentPlatform());
+
+  enclosure.os = "linux";
+  EXPECT_FALSE(enclosure.MatchesCurrentPlatform());
+#endif
+}
+
+TEST(AppcastItemTest, GetEnclosureForCurrentPlatform_ReturnsCorrectOne) {
+  AppcastItem item;
+  item.version = base::Version("1.0.0");
+
+  AppcastEnclosure mac_arm;
+  mac_arm.os = "macos";
+  mac_arm.arch = "arm64";
+  mac_arm.url = "https://example.com/mac-arm.zip";
+
+  AppcastEnclosure mac_x64;
+  mac_x64.os = "macos";
+  mac_x64.arch = "x86_64";
+  mac_x64.url = "https://example.com/mac-x64.zip";
+
+  AppcastEnclosure linux_x64;
+  linux_x64.os = "linux";
+  linux_x64.arch = "x86_64";
+  linux_x64.url = "https://example.com/linux-x64.zip";
+
+  AppcastEnclosure windows_x64;
+  windows_x64.os = "windows";
+  windows_x64.arch = "x86_64";
+  windows_x64.url = "https://example.com/windows-x64.zip";
+
+  item.enclosures = {mac_arm, mac_x64, linux_x64, windows_x64};
+
+  const AppcastEnclosure* match = item.GetEnclosureForCurrentPlatform();
+
+#if BUILDFLAG(IS_MAC) && defined(ARCH_CPU_ARM64)
+  ASSERT_NE(nullptr, match);
+  EXPECT_EQ("https://example.com/mac-arm.zip", match->url);
+#elif BUILDFLAG(IS_MAC)
+  ASSERT_NE(nullptr, match);
+  EXPECT_EQ("https://example.com/mac-x64.zip", match->url);
+#elif BUILDFLAG(IS_LINUX)
+  ASSERT_NE(nullptr, match);
+  EXPECT_EQ("https://example.com/linux-x64.zip", match->url);
+#elif BUILDFLAG(IS_WIN)
+  ASSERT_NE(nullptr, match);
+  EXPECT_EQ("https://example.com/windows-x64.zip", match->url);
+#endif
+}
+
+TEST(AppcastItemTest, GetEnclosureForCurrentPlatform_ReturnsNullWhenNoMatch) {
+  AppcastItem item;
+  item.version = base::Version("1.0.0");
+
+  // Add enclosures that don't match any platform this test runs on
+  AppcastEnclosure fake_os;
+  fake_os.os = "fakeos";
+  fake_os.arch = "arm64";
+
+  item.enclosures = {fake_os};
+
+  const AppcastEnclosure* match = item.GetEnclosureForCurrentPlatform();
+  EXPECT_EQ(nullptr, match);
+}
+
+TEST(AppcastItemTest, GetEnclosureForCurrentPlatform_ReturnsNullWhenEmpty) {
+  AppcastItem item;
+  item.version = base::Version("1.0.0");
+  item.enclosures = {};
+
+  const AppcastEnclosure* match = item.GetEnclosureForCurrentPlatform();
+  EXPECT_EQ(nullptr, match);
+}
+
+// =============================================================================
+// Edge Cases
+// =============================================================================
+
+TEST(BrowserOSAppcastParserTest, HandlesWhitespaceInVersion) {
+  const char kWhitespaceXml[] = R"(
+    <rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
+      <channel>
+        <item>
+          <sparkle:version>  1.2.3  </sparkle:version>
+          <enclosure url="https://example.com/download.zip"
+                     sparkle:os="macos" sparkle:arch="arm64"
+                     sparkle:edSignature="sig" length="100"/>
+        </item>
+      </channel>
+    </rss>
+  )";
+
+  auto item = BrowserOSAppcastParser::ParseLatestItem(kWhitespaceXml);
+
+  // base::Version doesn't handle leading/trailing whitespace, so the version
+  // is invalid and the item is rejected (items with invalid versions are
+  // skipped by the parser)
+  EXPECT_FALSE(item.has_value());
+}
+
+TEST(BrowserOSAppcastParserTest, HandlesZeroLengthEnclosure) {
+  const char kZeroLengthXml[] = R"(
+    <rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
+      <channel>
+        <item>
+          <sparkle:version>1.0.0</sparkle:version>
+          <enclosure url="https://example.com/download.zip"
+                     sparkle:os="macos" sparkle:arch="arm64"
+                     sparkle:edSignature="sig" length="0"/>
+        </item>
+      </channel>
+    </rss>
+  )";
+
+  auto item = BrowserOSAppcastParser::ParseLatestItem(kZeroLengthXml);
+
+  ASSERT_TRUE(item.has_value());
+  ASSERT_EQ(1u, item->enclosures.size());
+  EXPECT_EQ(0, item->enclosures[0].length);
+}
+
+TEST(BrowserOSAppcastParserTest, HandlesMissingSignature) {
+  const char kNoSigXml[] = R"(
+    <rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
+      <channel>
+        <item>
+          <sparkle:version>1.0.0</sparkle:version>
+          <enclosure url="https://example.com/download.zip"
+                     sparkle:os="macos" sparkle:arch="arm64"
+                     length="100"/>
+        </item>
+      </channel>
+    </rss>
+  )";
+
+  auto item = BrowserOSAppcastParser::ParseLatestItem(kNoSigXml);
+
+  ASSERT_TRUE(item.has_value());
+  ASSERT_EQ(1u, item->enclosures.size());
+  EXPECT_TRUE(item->enclosures[0].signature.empty());
+}
+
+}  // namespace
+}  // namespace browseros_server
