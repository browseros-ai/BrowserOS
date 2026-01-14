diff --git a/chrome/browser/importer/profile_writer.h b/chrome/browser/importer/profile_writer.h
index f609d99dde302..54119399b48f0 100644
--- a/chrome/browser/importer/profile_writer.h
+++ b/chrome/browser/importer/profile_writer.h
@@ -22,6 +22,10 @@ namespace autofill {
 class AutocompleteEntry;
 }
 
+namespace browseros_importer {
+struct ImportedCookieEntry;
+}  // namespace browseros_importer
+
 namespace password_manager {
 struct PasswordForm;
 }  // namespace password_manager
@@ -48,6 +52,8 @@ class ProfileWriter : public base::RefCountedThreadSafe<ProfileWriter> {
   // Helper methods for adding data to local stores.
   virtual void AddPasswordForm(const password_manager::PasswordForm& form);
 
+  virtual void AddCookie(const browseros_importer::ImportedCookieEntry& cookie);
+
   virtual void AddHistoryPage(const history::URLRows& page,
                               history::VisitSource visit_source);
 
@@ -92,6 +98,9 @@ class ProfileWriter : public base::RefCountedThreadSafe<ProfileWriter> {
   virtual void AddAutocompleteFormDataEntries(
       const std::vector<autofill::AutocompleteEntry>& autocomplete_entries);
 
+  // Adds the imported extensions to the profile.
+  virtual void AddExtensions(const std::vector<std::string>& extension_ids);
+
  protected:
   friend class base::RefCountedThreadSafe<ProfileWriter>;
 
