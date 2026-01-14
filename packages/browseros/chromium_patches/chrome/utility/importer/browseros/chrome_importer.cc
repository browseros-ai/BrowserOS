diff --git a/chrome/utility/importer/browseros/chrome_importer.cc b/chrome/utility/importer/browseros/chrome_importer.cc
new file mode 100644
index 0000000000000..56c8e8df5e21e
--- /dev/null
+++ b/chrome/utility/importer/browseros/chrome_importer.cc
@@ -0,0 +1,202 @@
+// Copyright 2023 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/utility/importer/browseros/chrome_importer.h"
+
+#include "base/logging.h"
+#include "chrome/common/importer/importer_bridge.h"
+#include "chrome/grit/generated_resources.h"
+#include "chrome/utility/importer/browseros/chrome_autofill_importer.h"
+#include "chrome/utility/importer/browseros/chrome_bookmarks_importer.h"
+#include "chrome/utility/importer/browseros/chrome_cookie_importer.h"
+#include "chrome/utility/importer/browseros/chrome_extensions_importer.h"
+#include "chrome/utility/importer/browseros/chrome_history_importer.h"
+#include "chrome/utility/importer/browseros/chrome_password_importer.h"
+#include "components/user_data_importer/common/importer_data_types.h"
+#include "ui/base/l10n/l10n_util.h"
+
+ChromeImporter::ChromeImporter() = default;
+
+ChromeImporter::~ChromeImporter() = default;
+
+void ChromeImporter::StartImport(
+    const user_data_importer::SourceProfile& source_profile,
+    uint16_t items,
+    ImporterBridge* bridge) {
+  bridge_ = bridge;
+  source_path_ = source_profile.source_path;
+
+  bridge_->NotifyStarted();
+
+  if ((items & user_data_importer::HISTORY) && !cancelled()) {
+    bridge_->NotifyItemStarted(user_data_importer::HISTORY);
+    ImportHistory();
+    bridge_->NotifyItemEnded(user_data_importer::HISTORY);
+  }
+
+  if ((items & user_data_importer::FAVORITES) && !cancelled()) {
+    bridge_->NotifyItemStarted(user_data_importer::FAVORITES);
+    ImportBookmarks();
+    bridge_->NotifyItemEnded(user_data_importer::FAVORITES);
+  }
+
+  if ((items & user_data_importer::PASSWORDS) && !cancelled()) {
+    bridge_->NotifyItemStarted(user_data_importer::PASSWORDS);
+    ImportPasswords();
+    bridge_->NotifyItemEnded(user_data_importer::PASSWORDS);
+  }
+
+  if ((items & user_data_importer::COOKIES) && !cancelled()) {
+    bridge_->NotifyItemStarted(user_data_importer::COOKIES);
+    ImportCookies();
+    bridge_->NotifyItemEnded(user_data_importer::COOKIES);
+  }
+
+  if ((items & user_data_importer::AUTOFILL_FORM_DATA) && !cancelled()) {
+    bridge_->NotifyItemStarted(user_data_importer::AUTOFILL_FORM_DATA);
+    ImportAutofillFormData();
+    bridge_->NotifyItemEnded(user_data_importer::AUTOFILL_FORM_DATA);
+  }
+
+  if ((items & user_data_importer::EXTENSIONS) && !cancelled()) {
+    bridge_->NotifyItemStarted(user_data_importer::EXTENSIONS);
+    ImportExtensions();
+    bridge_->NotifyItemEnded(user_data_importer::EXTENSIONS);
+  }
+
+  bridge_->NotifyEnded();
+}
+
+void ChromeImporter::ImportHistory() {
+  LOG(INFO) << "ChromeImporter: Starting history import";
+
+  std::vector<user_data_importer::ImporterURLRow> rows =
+      browseros_importer::ImportChromeHistory(source_path_);
+
+  if (rows.empty()) {
+    LOG(INFO) << "ChromeImporter: No history to import";
+    return;
+  }
+
+  LOG(INFO) << "ChromeImporter: Importing " << rows.size() << " history items";
+
+  if (!cancelled()) {
+    bridge_->SetHistoryItems(rows,
+                             user_data_importer::VISIT_SOURCE_CHROME_IMPORTED);
+  }
+
+  LOG(INFO) << "ChromeImporter: History import complete";
+}
+
+void ChromeImporter::ImportBookmarks() {
+  LOG(INFO) << "ChromeImporter: Starting bookmarks import";
+
+  browseros_importer::ChromeBookmarksResult result =
+      browseros_importer::ImportChromeBookmarks(source_path_);
+
+  if (!result.bookmarks.empty() && !cancelled()) {
+    LOG(INFO) << "ChromeImporter: Importing " << result.bookmarks.size()
+              << " bookmarks";
+    bridge_->AddBookmarks(result.bookmarks,
+                          l10n_util::GetStringUTF16(IDS_IMPORT_FROM_CHROME));
+  } else {
+    LOG(INFO) << "ChromeImporter: No bookmarks to import";
+  }
+
+  if (!result.favicons.empty() && !cancelled()) {
+    LOG(INFO) << "ChromeImporter: Importing " << result.favicons.size()
+              << " favicons";
+    bridge_->SetFavicons(result.favicons);
+  }
+
+  LOG(INFO) << "ChromeImporter: Bookmarks import complete";
+}
+
+void ChromeImporter::ImportPasswords() {
+  LOG(INFO) << "ChromeImporter: Starting password import";
+
+  std::vector<user_data_importer::ImportedPasswordForm> passwords =
+      browseros_importer::ImportChromePasswords(source_path_);
+
+  if (passwords.empty()) {
+    LOG(INFO) << "ChromeImporter: No passwords to import";
+    return;
+  }
+
+  LOG(INFO) << "ChromeImporter: Importing " << passwords.size() << " passwords";
+
+  for (const auto& password : passwords) {
+    if (cancelled()) {
+      break;
+    }
+    bridge_->SetPasswordForm(password);
+  }
+
+  LOG(INFO) << "ChromeImporter: Password import complete";
+}
+
+void ChromeImporter::ImportCookies() {
+  LOG(INFO) << "ChromeImporter: Starting cookie import";
+
+  std::vector<browseros_importer::ImportedCookieEntry> cookies =
+      browseros_importer::ImportChromeCookies(source_path_);
+
+  if (cookies.empty()) {
+    LOG(INFO) << "ChromeImporter: No cookies to import";
+    return;
+  }
+
+  LOG(INFO) << "ChromeImporter: Importing " << cookies.size() << " cookies";
+
+  for (const auto& cookie : cookies) {
+    if (cancelled()) {
+      break;
+    }
+    bridge_->SetCookie(cookie);
+  }
+
+  LOG(INFO) << "ChromeImporter: Cookie import complete";
+}
+
+void ChromeImporter::ImportAutofillFormData() {
+  LOG(INFO) << "ChromeImporter: Starting autofill import";
+
+  std::vector<ImporterAutofillFormDataEntry> entries =
+      browseros_importer::ImportChromeAutofill(source_path_);
+
+  if (entries.empty()) {
+    LOG(INFO) << "ChromeImporter: No autofill entries to import";
+    return;
+  }
+
+  LOG(INFO) << "ChromeImporter: Importing " << entries.size()
+            << " autofill entries";
+
+  if (!cancelled()) {
+    bridge_->SetAutofillFormData(entries);
+  }
+
+  LOG(INFO) << "ChromeImporter: Autofill import complete";
+}
+
+void ChromeImporter::ImportExtensions() {
+  LOG(INFO) << "ChromeImporter: Starting extensions import";
+
+  std::vector<std::string> extension_ids =
+      browseros_importer::ImportChromeExtensions(source_path_);
+
+  if (extension_ids.empty()) {
+    LOG(INFO) << "ChromeImporter: No extensions to import";
+    return;
+  }
+
+  LOG(INFO) << "ChromeImporter: Importing " << extension_ids.size()
+            << " extensions";
+
+  if (!cancelled()) {
+    bridge_->SetExtensions(extension_ids);
+  }
+
+  LOG(INFO) << "ChromeImporter: Extensions import complete";
+}
