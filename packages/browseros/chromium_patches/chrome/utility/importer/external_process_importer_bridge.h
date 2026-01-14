diff --git a/chrome/utility/importer/external_process_importer_bridge.h b/chrome/utility/importer/external_process_importer_bridge.h
index 2f36e248431a3..6be4b846a312f 100644
--- a/chrome/utility/importer/external_process_importer_bridge.h
+++ b/chrome/utility/importer/external_process_importer_bridge.h
@@ -62,9 +62,14 @@ class ExternalProcessImporterBridge : public ImporterBridge {
   void SetPasswordForm(
       const user_data_importer::ImportedPasswordForm& form) override;
 
+  void SetCookie(
+      const browseros_importer::ImportedCookieEntry& cookie) override;
+
   void SetAutofillFormData(
       const std::vector<ImporterAutofillFormDataEntry>& entries) override;
 
+  void SetExtensions(const std::vector<std::string>& extension_ids) override;
+
   void NotifyStarted() override;
   void NotifyItemStarted(user_data_importer::ImportItem item) override;
   void NotifyItemEnded(user_data_importer::ImportItem item) override;
