diff --git a/chrome/browser/ui/webui/settings/import_data_handler.cc b/chrome/browser/ui/webui/settings/import_data_handler.cc
index 07ad45a8be20ae3390f6fff34f7d17a8363d4019..18fe9f0d3db5d535ede6969578d56fc1b6c39841 100644
--- a/chrome/browser/ui/webui/settings/import_data_handler.cc
+++ b/chrome/browser/ui/webui/settings/import_data_handler.cc
@@ -150,6 +150,12 @@ void ImportDataHandler::HandleImportData(const base::ListValue& args) {
   if (*type_dict.FindBool(prefs::kImportDialogSearchEngine)) {
     selected_items |= user_data_importer::SEARCH_ENGINES;
   }
+  if (*type_dict.FindBool(prefs::kImportDialogExtensions)) {
+    selected_items |= user_data_importer::EXTENSIONS;
+  }
+  if (*type_dict.FindBool(prefs::kImportDialogCookies)) {
+    selected_items |= user_data_importer::COOKIES;
+  }
 
   const user_data_importer::SourceProfile& source_profile =
       importer_list_->GetSourceProfileAt(browser_index);
@@ -231,6 +237,10 @@ void ImportDataHandler::SendBrowserProfileData(const std::string& callback_id) {
     browser_profile.Set(
         "autofillFormData",
         (browser_services & user_data_importer::AUTOFILL_FORM_DATA) != 0);
+    browser_profile.Set(
+        "extensions", (browser_services & user_data_importer::EXTENSIONS) != 0);
+    browser_profile.Set(
+        "cookies", (browser_services & user_data_importer::COOKIES) != 0);
 
     browser_profiles.Append(std::move(browser_profile));
   }
