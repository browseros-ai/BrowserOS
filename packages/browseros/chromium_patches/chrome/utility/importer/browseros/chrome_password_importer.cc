diff --git a/chrome/utility/importer/browseros/chrome_password_importer.cc b/chrome/utility/importer/browseros/chrome_password_importer.cc
new file mode 100644
index 0000000000000..cbae752f359b5
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_password_importer.cc
@@ -0,0 +1,151 @@
+// Copyright 2024 AKW Technology Inc
+// Chrome password importer implementation
+
+#include "chrome/utility/importer/browseros/chrome_password_importer.h"
+
+#include "base/files/file_util.h"
+#include "base/logging.h"
+#include "base/strings/utf_string_conversions.h"
+#include "chrome/utility/importer/browseros/chrome_decryptor.h"
+#include "sql/database.h"
+#include "sql/statement.h"
+#include "url/gurl.h"
+
+namespace browseros_importer {
+
+namespace {
+
+// Database tag - reuse ChromeImporter tag which is registered in histograms.xml
+inline constexpr sql::Database::Tag kDatabaseTag{"ChromeImporter"};
+
+constexpr char kLoginDataFilename[] = "Login Data";
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
+}  // namespace
+
+std::vector<user_data_importer::ImportedPasswordForm> ImportChromePasswords(
+    const base::FilePath& profile_path) {
+  std::vector<user_data_importer::ImportedPasswordForm> passwords;
+
+  // Extract encryption key
+  KeyExtractionResult key_result;
+  std::string encryption_key = ExtractChromeKey(profile_path, &key_result);
+
+  if (encryption_key.empty()) {
+    LOG(WARNING) << "browseros: Failed to extract encryption key, "
+                 << "result: " << static_cast<int>(key_result);
+    return passwords;
+  }
+
+  // Path to Login Data database
+  base::FilePath login_data_path = profile_path.AppendASCII(kLoginDataFilename);
+  if (!base::PathExists(login_data_path)) {
+    LOG(WARNING) << "browseros: Login Data not found at: "
+                 << login_data_path.value();
+    return passwords;
+  }
+
+  // Copy to temp location to avoid locking issues
+  base::FilePath temp_db_path = CopyDatabaseToTemp(login_data_path);
+  if (temp_db_path.empty()) {
+    return passwords;
+  }
+
+  // Open database
+  sql::Database db(kDatabaseTag);
+  if (!db.Open(temp_db_path)) {
+    LOG(WARNING) << "browseros: Failed to open Login Data database";
+    base::DeleteFile(temp_db_path);
+    return passwords;
+  }
+
+  // Query logins table - use scope block to ensure statement is destroyed before
+  // db.Close() to avoid DCHECK failure
+  {
+    const char kQuery[] =
+        "SELECT origin_url, action_url, username_element, username_value, "
+        "password_element, password_value, signon_realm, blacklisted_by_user, "
+        "scheme FROM logins";
+
+    sql::Statement statement(db.GetUniqueStatement(kQuery));
+    if (!statement.is_valid()) {
+      LOG(WARNING) << "browseros: Failed to prepare query";
+      base::DeleteFile(temp_db_path);
+      return passwords;
+    }
+
+    while (statement.Step()) {
+      std::string origin_url = statement.ColumnString(0);
+      std::string action_url = statement.ColumnString(1);
+      std::u16string username_element = statement.ColumnString16(2);
+      std::u16string username_value = statement.ColumnString16(3);
+      std::u16string password_element = statement.ColumnString16(4);
+
+      // password_value is a BLOB - encrypted
+      std::string encrypted_password;
+      statement.ColumnBlobAsString(5, &encrypted_password);
+
+      std::string signon_realm = statement.ColumnString(6);
+      bool blacklisted = statement.ColumnBool(7);
+      int scheme = statement.ColumnInt(8);
+
+      // Decrypt password
+      std::string decrypted_password;
+      if (!encrypted_password.empty()) {
+        if (!DecryptChromeValue(encrypted_password, encryption_key,
+                                &decrypted_password)) {
+          LOG(WARNING) << "browseros: Failed to decrypt password for: "
+                       << origin_url;
+          continue;
+        }
+      }
+
+      // Create ImportedPasswordForm
+      user_data_importer::ImportedPasswordForm form;
+
+      // Set scheme
+      if (scheme == 0) {
+        form.scheme = user_data_importer::ImportedPasswordForm::Scheme::kHtml;
+      } else {
+        form.scheme = user_data_importer::ImportedPasswordForm::Scheme::kBasic;
+      }
+
+      form.signon_realm = signon_realm;
+      form.url = GURL(origin_url);
+      form.action = GURL(action_url);
+      form.username_element = username_element;
+      form.username_value = username_value;
+      form.password_element = password_element;
+      form.password_value = base::UTF8ToUTF16(decrypted_password);
+      form.blocked_by_user = blacklisted;
+
+      passwords.push_back(std::move(form));
+    }
+  }  // statement destroyed here
+
+  db.Close();
+  base::DeleteFile(temp_db_path);
+
+  LOG(INFO) << "browseros: Imported " << passwords.size()
+            << " passwords";
+
+  return passwords;
+}
+
+}  // namespace browseros_importer
