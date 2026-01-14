diff --git a/chrome/browser/importer/external_process_importer_client.h b/chrome/browser/importer/external_process_importer_client.h
index 42b466d3ce66b..eaa231f2015c3 100644
--- a/chrome/browser/importer/external_process_importer_client.h
+++ b/chrome/browser/importer/external_process_importer_client.h
@@ -73,6 +73,8 @@ class ExternalProcessImporterClient
       const favicon_base::FaviconUsageDataList& favicons_group) override;
   void OnPasswordFormImportReady(
       const user_data_importer::ImportedPasswordForm& form) override;
+  void OnCookieImportReady(
+      chrome::mojom::ImportedCookieEntryPtr cookie) override;
   void OnKeywordsImportReady(
       const std::vector<user_data_importer::SearchEngineInfo>& search_engines,
       bool unique_on_host_and_path) override;
@@ -81,6 +83,8 @@ class ExternalProcessImporterClient
   void OnAutofillFormDataImportGroup(
       const std::vector<ImporterAutofillFormDataEntry>&
           autofill_form_data_entry_group) override;
+  void OnExtensionsImportReady(
+      const std::vector<std::string>& extension_ids) override;
 
  protected:
   ~ExternalProcessImporterClient() override;
