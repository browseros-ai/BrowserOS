diff --git a/components/password_manager/core/browser/password_store/password_store_backend.h b/components/password_manager/core/browser/password_store/password_store_backend.h
index 175b2ddfc66d76a9669614d6a848d713ec48d25f..ae301860990b21ecd4dbcd57ad061fd23fd2a845 100644
--- a/components/password_manager/core/browser/password_store/password_store_backend.h
+++ b/components/password_manager/core/browser/password_store/password_store_backend.h
@@ -78,6 +78,20 @@ class PasswordStoreBackend {
   // status). Callback is called on the main sequence.
   virtual void GetAllLoginsAsync(BackendLoginsOrErrorReply callback) = 0;
 
+  // Strict operations fail closed unless a backend explicitly guarantees them.
+  // Ordinary UI retrieval retains its existing partial-result behavior.
+  virtual void GetAllLoginsStrictAsync(BackendLoginsOrErrorReply callback);
+  virtual void FillMatchingLoginsStrictAsync(
+      BackendLoginsOrErrorReply callback,
+      bool include_psl,
+      const std::vector<PasswordFormDigest>& forms);
+  virtual void GetGroupedMatchingLoginsStrictAsync(
+      const PasswordFormDigest& form,
+      BackendLoginsOrErrorReply callback);
+  virtual void AddLoginIfAbsentAsync(
+      StoredCredential cred,
+      base::OnceCallback<void(PasswordStoreAddResult)> callback);
+
   // Returns the complete list of credentials and fills in affiliation and
   // branding information for Android credentials. Callback is called on the
   // main sequence.
