diff --git a/components/password_manager/core/browser/password_store/password_store_built_in_backend.h b/components/password_manager/core/browser/password_store/password_store_built_in_backend.h
index 7da00c78cb9db39068484f384a2d0b71c5ed1ced..c6f2d150663b2737276c8de4e59e770cdf6496f3 100644
--- a/components/password_manager/core/browser/password_store/password_store_built_in_backend.h
+++ b/components/password_manager/core/browser/password_store/password_store_built_in_backend.h
@@ -80,6 +80,17 @@ class PasswordStoreBuiltInBackend : public PasswordStoreBackend,
   void Shutdown(base::OnceClosure shutdown_completed) override;
   ActionableError GetError() override;
   void GetAllLoginsAsync(BackendLoginsOrErrorReply callback) override;
+  void GetAllLoginsStrictAsync(BackendLoginsOrErrorReply callback) override;
+  void FillMatchingLoginsStrictAsync(
+      BackendLoginsOrErrorReply callback,
+      bool include_psl,
+      const std::vector<PasswordFormDigest>& forms) override;
+  void GetGroupedMatchingLoginsStrictAsync(
+      const PasswordFormDigest& form,
+      BackendLoginsOrErrorReply callback) override;
+  void AddLoginIfAbsentAsync(
+      StoredCredential cred,
+      base::OnceCallback<void(PasswordStoreAddResult)> callback) override;
   void GetAllLoginsWithAffiliationAndBrandingAsync(
       BackendLoginsOrErrorReply callback) override;
   void GetAutofillableLoginsAsync(BackendLoginsOrErrorReply callback) override;
