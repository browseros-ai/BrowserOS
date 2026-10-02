diff --git a/chrome/browser/importer/in_process_importer_bridge.cc b/chrome/browser/importer/in_process_importer_bridge.cc
index da614f0e7db5628e258f07fe0d7f2967dc1207f1..0c546a716728802b1cd038539e0f05b423fb1176 100644
--- a/chrome/browser/importer/in_process_importer_bridge.cc
+++ b/chrome/browser/importer/in_process_importer_bridge.cc
@@ -8,6 +8,7 @@
 
 #include <iterator>
 
+#include "base/logging.h"
 #include "base/strings/string_util.h"
 #include "base/strings/utf_string_conversions.h"
 #include "build/build_config.h"
@@ -21,11 +22,16 @@
 #include "components/search_engines/template_url.h"
 #include "components/search_engines/template_url_parser.h"
 #include "components/search_engines/template_url_prepopulate_data.h"
+#include "chrome/utility/importer/browseros/chrome_cookie_importer.h"
 #include "components/user_data_importer/common/imported_bookmark_entry.h"
 #include "ui/base/l10n/l10n_util.h"
 
 namespace {
 
+// Temporary definition for Chrome imported visits, mapped to value 4
+const history::VisitSource SOURCE_CHROME_IMPORTED =
+    static_cast<history::VisitSource>(4);
+
 history::URLRows ConvertImporterURLRowsToHistoryURLRows(
     const std::vector<user_data_importer::ImporterURLRow>& rows) {
   history::URLRows converted;
@@ -53,6 +59,8 @@ history::VisitSource ConvertImporterVisitSourceToHistoryVisitSource(
       return history::SOURCE_IE_IMPORTED;
     case user_data_importer::VISIT_SOURCE_SAFARI_IMPORTED:
       return history::SOURCE_SAFARI_IMPORTED;
+    case user_data_importer::VISIT_SOURCE_CHROME_IMPORTED:
+      return SOURCE_CHROME_IMPORTED;
   }
   NOTREACHED();
 }
@@ -155,6 +163,11 @@ void InProcessImporterBridge::SetPasswordForm(
   writer_->AddPasswordForm(ConvertImportedPasswordForm(form));
 }
 
+void InProcessImporterBridge::SetCookie(
+    const browseros_importer::ImportedCookieEntry& cookie) {
+  writer_->AddCookie(cookie);
+}
+
 void InProcessImporterBridge::SetAutofillFormData(
     const std::vector<ImporterAutofillFormDataEntry>& entries) {
   std::vector<autofill::AutocompleteEntry> autocomplete_entries;
@@ -172,6 +185,15 @@ void InProcessImporterBridge::SetAutofillFormData(
   writer_->AddAutocompleteFormDataEntries(autocomplete_entries);
 }
 
+void InProcessImporterBridge::SetExtensions(
+    const std::vector<std::string>& extension_ids) {
+  LOG(INFO) << "InProcessImporterBridge: Received " << extension_ids.size()
+            << " extensions to import";
+
+  // Pass the extension IDs to the profile writer to handle installation
+  writer_->AddExtensions(extension_ids);
+}
+
 void InProcessImporterBridge::NotifyStarted() {
   host_->NotifyImportStarted();
 }
