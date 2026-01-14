diff --git a/chrome/browser/importer/in_process_importer_bridge.cc b/chrome/browser/importer/in_process_importer_bridge.cc
index fbb20f1ae0668..2ae55dd11704a 100644
--- a/chrome/browser/importer/in_process_importer_bridge.cc
+++ b/chrome/browser/importer/in_process_importer_bridge.cc
@@ -21,11 +21,16 @@
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
@@ -53,6 +58,8 @@ history::VisitSource ConvertImporterVisitSourceToHistoryVisitSource(
       return history::SOURCE_IE_IMPORTED;
     case user_data_importer::VISIT_SOURCE_SAFARI_IMPORTED:
       return history::SOURCE_SAFARI_IMPORTED;
+    case user_data_importer::VISIT_SOURCE_CHROME_IMPORTED:
+      return SOURCE_CHROME_IMPORTED;
   }
   NOTREACHED();
 }
@@ -151,6 +158,11 @@ void InProcessImporterBridge::SetPasswordForm(
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
@@ -168,6 +180,15 @@ void InProcessImporterBridge::SetAutofillFormData(
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
