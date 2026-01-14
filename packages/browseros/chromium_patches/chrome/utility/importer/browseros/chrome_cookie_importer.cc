diff --git a/chrome/utility/importer/browseros/chrome_cookie_importer.cc b/chrome/utility/importer/browseros/chrome_cookie_importer.cc
new file mode 100644
index 0000000000000..4497532338b3f
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_cookie_importer.cc
@@ -0,0 +1,229 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome cookie importer implementation
+
+#include "chrome/utility/importer/browseros/chrome_cookie_importer.h"
+
+#include "base/files/file_util.h"
+#include "base/logging.h"
+#include "chrome/utility/importer/browseros/chrome_decryptor.h"
+#include "sql/database.h"
+#include "sql/statement.h"
+
+namespace browseros_importer {
+
+ImportedCookieEntry::ImportedCookieEntry() = default;
+ImportedCookieEntry::ImportedCookieEntry(const ImportedCookieEntry& other) =
+    default;
+ImportedCookieEntry::ImportedCookieEntry(ImportedCookieEntry&& other) noexcept =
+    default;
+ImportedCookieEntry& ImportedCookieEntry::operator=(
+    const ImportedCookieEntry& other) = default;
+ImportedCookieEntry& ImportedCookieEntry::operator=(
+    ImportedCookieEntry&& other) noexcept = default;
+ImportedCookieEntry::~ImportedCookieEntry() = default;
+
+namespace {
+
+// Database tag - reuse ChromeImporter tag which is registered in histograms.xml
+inline constexpr sql::Database::Tag kDatabaseTag{"ChromeImporter"};
+
+constexpr char kCookiesFilename[] = "Cookies";
+
+// Copy database to temp location to avoid locking issues with Chrome
+base::FilePath CopyDatabaseToTemp(const base::FilePath& db_path) {
+  base::FilePath temp_path;
+  if (!base::CreateTemporaryFile(&temp_path)) {
+    LOG(WARNING) << "browseros: Failed to create temp file";
+    return base::FilePath();
+  }
+
+  if (!base::CopyFile(db_path, temp_path)) {
+    LOG(WARNING) << "browseros: Failed to copy database to temp";
+    base::DeleteFile(temp_path);
+    return base::FilePath();
+  }
+
+  return temp_path;
+}
+
+// Convert Chrome's microseconds since Windows epoch to base::Time
+// Chrome stores times as microseconds since Jan 1, 1601 (Windows epoch)
+base::Time ChromeTimeToBaseTime(int64_t chrome_time) {
+  if (chrome_time == 0) {
+    return base::Time();
+  }
+  // base::Time::FromDeltaSinceWindowsEpoch handles the conversion
+  return base::Time::FromDeltaSinceWindowsEpoch(
+      base::Microseconds(chrome_time));
+}
+
+// Map Chrome's samesite integer to net::CookieSameSite
+net::CookieSameSite IntToSameSite(int value) {
+  switch (value) {
+    case 0:
+      return net::CookieSameSite::NO_RESTRICTION;
+    case 1:
+      return net::CookieSameSite::LAX_MODE;
+    case 2:
+      return net::CookieSameSite::STRICT_MODE;
+    default:
+      return net::CookieSameSite::UNSPECIFIED;
+  }
+}
+
+// Map Chrome's priority integer to net::CookiePriority
+net::CookiePriority IntToPriority(int value) {
+  switch (value) {
+    case 0:
+      return net::CookiePriority::COOKIE_PRIORITY_LOW;
+    case 1:
+      return net::CookiePriority::COOKIE_PRIORITY_MEDIUM;
+    case 2:
+      return net::CookiePriority::COOKIE_PRIORITY_HIGH;
+    default:
+      return net::CookiePriority::COOKIE_PRIORITY_MEDIUM;
+  }
+}
+
+// Map Chrome's source_scheme integer to net::CookieSourceScheme
+net::CookieSourceScheme IntToSourceScheme(int value) {
+  switch (value) {
+    case 0:
+      return net::CookieSourceScheme::kUnset;
+    case 1:
+      return net::CookieSourceScheme::kNonSecure;
+    case 2:
+      return net::CookieSourceScheme::kSecure;
+    default:
+      return net::CookieSourceScheme::kUnset;
+  }
+}
+
+}  // namespace
+
+std::vector<ImportedCookieEntry> ImportChromeCookies(
+    const base::FilePath& profile_path) {
+  std::vector<ImportedCookieEntry> cookies;
+
+  // Extract encryption key (same key used for passwords and cookies)
+  KeyExtractionResult key_result;
+  std::string encryption_key = ExtractChromeKey(profile_path, &key_result);
+
+  if (encryption_key.empty()) {
+    LOG(WARNING) << "browseros: Failed to extract encryption key, "
+                 << "result: " << static_cast<int>(key_result);
+    return cookies;
+  }
+
+  // Path to Cookies database
+  base::FilePath cookies_path = profile_path.AppendASCII(kCookiesFilename);
+  if (!base::PathExists(cookies_path)) {
+    LOG(WARNING) << "browseros: Cookies not found at: "
+                 << cookies_path.value();
+    return cookies;
+  }
+
+  // Copy to temp location to avoid locking issues
+  base::FilePath temp_db_path = CopyDatabaseToTemp(cookies_path);
+  if (temp_db_path.empty()) {
+    return cookies;
+  }
+
+  // Open database
+  sql::Database db(kDatabaseTag);
+  if (!db.Open(temp_db_path)) {
+    LOG(WARNING) << "browseros: Failed to open Cookies database";
+    base::DeleteFile(temp_db_path);
+    return cookies;
+  }
+
+  // Check database version - Chrome 130+ (version ≥ 24) prepends SHA256 hash
+  // to cookie values before encryption. We need to strip this 32-byte prefix.
+  int db_version = 0;
+  {
+    sql::Statement version_stmt(
+        db.GetUniqueStatement("SELECT value FROM meta WHERE key = 'version'"));
+    if (version_stmt.Step()) {
+      db_version = version_stmt.ColumnInt(0);
+    }
+  }
+
+  // SHA256 hash is 32 bytes
+  constexpr size_t kSha256HashLength = 32;
+  const bool has_domain_hash_prefix = (db_version >= 24);
+
+  // Query cookies table - use scope block to ensure statement is destroyed
+  // before db.Close() to avoid DCHECK failure
+  {
+    // Query all relevant columns from cookies table (v24+ schema)
+    const char kQuery[] =
+        "SELECT host_key, name, value, encrypted_value, path, expires_utc, "
+        "is_secure, is_httponly, creation_utc, last_access_utc, "
+        "samesite, priority, source_scheme, source_port, is_persistent, "
+        "last_update_utc "
+        "FROM cookies";
+
+    sql::Statement statement(db.GetUniqueStatement(kQuery));
+    if (!statement.is_valid()) {
+      LOG(WARNING) << "browseros: Failed to prepare query";
+      base::DeleteFile(temp_db_path);
+      return cookies;
+    }
+
+    while (statement.Step()) {
+      ImportedCookieEntry entry;
+
+      entry.host_key = statement.ColumnString(0);
+      entry.name = statement.ColumnString(1);
+
+      // Get both plaintext and encrypted values
+      std::string plaintext_value = statement.ColumnString(2);
+      std::string encrypted_value;
+      statement.ColumnBlobAsString(3, &encrypted_value);
+
+      // Prefer encrypted_value if present, otherwise use plaintext
+      if (!encrypted_value.empty()) {
+        std::string decrypted_value;
+        if (DecryptChromeValue(encrypted_value, encryption_key,
+                               &decrypted_value)) {
+          // Chrome 130+ (db version ≥ 24) prepends SHA256 hash of domain
+          // to the cookie value before encryption. Strip it after decryption.
+          if (has_domain_hash_prefix &&
+              decrypted_value.size() > kSha256HashLength) {
+            entry.value = decrypted_value.substr(kSha256HashLength);
+          } else {
+            entry.value = decrypted_value;
+          }
+        } else {
+          // Fall back to plaintext if decryption fails
+          entry.value = plaintext_value;
+        }
+      } else {
+        entry.value = plaintext_value;
+      }
+
+      entry.path = statement.ColumnString(4);
+      entry.expires_utc = ChromeTimeToBaseTime(statement.ColumnInt64(5));
+      entry.is_secure = statement.ColumnBool(6);
+      entry.is_httponly = statement.ColumnBool(7);
+      entry.creation_utc = ChromeTimeToBaseTime(statement.ColumnInt64(8));
+      entry.last_access_utc = ChromeTimeToBaseTime(statement.ColumnInt64(9));
+      entry.same_site = IntToSameSite(statement.ColumnInt(10));
+      entry.priority = IntToPriority(statement.ColumnInt(11));
+      entry.source_scheme = IntToSourceScheme(statement.ColumnInt(12));
+      entry.source_port = statement.ColumnInt(13);
+      entry.is_persistent = statement.ColumnBool(14);
+      entry.last_update_utc = ChromeTimeToBaseTime(statement.ColumnInt64(15));
+
+      cookies.push_back(std::move(entry));
+    }
+
+  }  // statement destroyed here
+
+  db.Close();
+  base::DeleteFile(temp_db_path);
+
+  return cookies;
+}
+
+}  // namespace browseros_importer
