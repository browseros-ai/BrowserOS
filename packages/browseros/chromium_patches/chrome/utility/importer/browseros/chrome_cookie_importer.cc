diff --git a/chrome/utility/importer/browseros/chrome_cookie_importer.cc b/chrome/utility/importer/browseros/chrome_cookie_importer.cc
new file mode 100644
index 0000000000000..21b17a08e1a8e
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_cookie_importer.cc
@@ -0,0 +1,295 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome cookie importer implementation
+
+#include "chrome/utility/importer/browseros/chrome_cookie_importer.h"
+
+#include <string_view>
+
+#include "base/files/file_util.h"
+#include "base/logging.h"
+#include "chrome/utility/importer/browseros/chrome_decryptor.h"
+#include "chrome/utility/importer/browseros/chrome_importer_utils.h"
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
+// Returns true if |domain| matches |target| exactly or is a subdomain of it.
+// E.g., DomainMatchesTarget("accounts.google.com", "google.com") -> true
+//       DomainMatchesTarget("google.com", "google.com") -> true
+//       DomainMatchesTarget("notgoogle.com", "google.com") -> false
+bool DomainMatchesTarget(std::string_view domain, std::string_view target) {
+  if (domain == target) {
+    return true;
+  }
+  // Check subdomain: domain must end with ".target"
+  if (domain.size() > target.size() + 1 &&
+      domain[domain.size() - target.size() - 1] == '.' &&
+      domain.substr(domain.size() - target.size()) == target) {
+    return true;
+  }
+  return false;
+}
+
+// Normalize host_key by stripping the leading dot (if any) for comparison.
+std::string_view NormalizeDomain(std::string_view host_key) {
+  if (!host_key.empty() && host_key[0] == '.') {
+    host_key.remove_prefix(1);
+  }
+  return host_key;
+}
+
+// Device-Bound Session Credential (DBSC) cookies. These cookies are
+// cryptographically tied to local key material and session state stored
+// separately (bound_session_params_storage.cc). Importing them without
+// the binding context causes auth failures after Google's token rotation
+// (~12-24h). They must be skipped.
+struct BoundCookieEntry {
+  std::string_view domain;
+  std::string_view name;
+};
+
+[[maybe_unused]]
+constexpr BoundCookieEntry kBoundSessionCookies[] = {
+    {"google.com", "__Secure-1PSIDTS"},
+    {"google.com", "__Secure-3PSIDTS"},
+};
+
+// Domains whose cookies should never be imported. Cookies from these domains
+// rely on state that cannot be migrated (device binding, token rotation, etc.)
+// and will break after import. Add domains here to blacklist them.
+constexpr std::string_view kBlacklistedDomains[] = {
+    // Example: "example.com" would block all cookies on example.com and
+    // *.example.com. Currently empty; add entries as needed.
+};
+
+bool ShouldSkipCookie(std::string_view host_key, std::string_view name) {
+  std::string_view domain = NormalizeDomain(host_key);
+
+  // Check device-bound session cookies.
+  for (const auto& entry : kBoundSessionCookies) {
+    if (name == entry.name && DomainMatchesTarget(domain, entry.domain)) {
+      return true;
+    }
+  }
+
+  // Check domain blacklist.
+  for (const auto& blocked : kBlacklistedDomains) {
+    if (DomainMatchesTarget(domain, blocked)) {
+      return true;
+    }
+  }
+
+  return false;
+}
+
+}  // namespace
+
+std::vector<ImportedCookieEntry> ImportChromeCookies(
+    const base::FilePath& profile_path,
+    const std::string& encryption_key) {
+  std::vector<ImportedCookieEntry> cookies;
+  if (encryption_key.empty()) {
+    LOG(WARNING) << "browseros: Chrome encryption key unavailable; "
+                 << "skipping cookie import";
+    return cookies;
+  }
+
+  // Path to Cookies database
+  base::FilePath cookies_path = profile_path.AppendASCII(kCookiesFilename);
+  if (!base::PathExists(cookies_path)) {
+    LOG(WARNING) << "browseros: Cookies not found at: " << cookies_path.value();
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
+    DeleteDatabaseTemp(temp_db_path);
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
+  // Counters for cookies we cannot import (reported after the loop).
+  int skipped_app_bound = 0;
+  int failed_decrypt = 0;
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
+      DeleteDatabaseTemp(temp_db_path);
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
+      std::string encrypted_value = statement.ColumnBlobAsString(3);
+
+      // Prefer encrypted_value if present, otherwise use plaintext
+      if (!encrypted_value.empty()) {
+        std::string decrypted_value;
+        DecryptResult decrypt_result = DecryptChromeValue(
+            encrypted_value, encryption_key, &decrypted_value);
+        if (decrypt_result == DecryptResult::kAppBoundUnsupported) {
+          // Chrome 127+ App-Bound (v20) cookie. Its key is held by Chrome's
+          // SYSTEM elevation service, so we skip it instead of importing an
+          // undecryptable value.
+          ++skipped_app_bound;
+          continue;
+        }
+        if (decrypt_result != DecryptResult::kSuccess) {
+          // Genuine decryption failure: skip rather than store a junk value.
+          ++failed_decrypt;
+          continue;
+        }
+        // Chrome 130+ (db version ≥ 24) prepends the SHA256 hash of the domain
+        // to the cookie value before encryption. Strip it after decryption.
+        if (has_domain_hash_prefix &&
+            decrypted_value.size() >= kSha256HashLength) {
+          entry.value = decrypted_value.substr(kSha256HashLength);
+        } else {
+          entry.value = decrypted_value;
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
+      // Skip cookies that cannot be safely migrated (device-bound session
+      // cookies, blacklisted domains).
+      if (ShouldSkipCookie(entry.host_key, entry.name)) {
+        VLOG(1) << "browseros: Skipping cookie " << entry.name << " on "
+                << entry.host_key << " (filtered)";
+        continue;
+      }
+
+      cookies.push_back(std::move(entry));
+    }
+
+  }  // statement destroyed here
+
+  db.Close();
+  DeleteDatabaseTemp(temp_db_path);
+
+  if (skipped_app_bound > 0 || failed_decrypt > 0) {
+    LOG(WARNING) << "browseros: Cookie import skipped " << skipped_app_bound
+                 << " App-Bound (v20) cookies and " << failed_decrypt
+                 << " undecryptable cookies; imported " << cookies.size()
+                 << ". App-Bound cookies (Chrome 127+) require Chrome's "
+                    "SYSTEM elevation service and cannot be imported.";
+  }
+
+  return cookies;
+}
+
+}  // namespace browseros_importer
