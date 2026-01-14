diff --git a/chrome/browser/importer/in_process_importer_bridge.h b/chrome/browser/importer/in_process_importer_bridge.h
index 61190844025f0..08ce2bd965704 100644
--- a/chrome/browser/importer/in_process_importer_bridge.h
+++ b/chrome/browser/importer/in_process_importer_bridge.h
@@ -49,9 +49,14 @@ class InProcessImporterBridge : public ImporterBridge {
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
