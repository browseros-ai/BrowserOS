diff --git a/components/password_manager/core/browser/password_store/password_store.h b/components/password_manager/core/browser/password_store/password_store.h
index 5c6dd53620f8168bdb96521711cd14bf34e3fb22..a6be6f22297236c72f098c1edf04e5aeeeb03e44 100644
--- a/components/password_manager/core/browser/password_store/password_store.h
+++ b/components/password_manager/core/browser/password_store/password_store.h
@@ -65,6 +65,13 @@ class PasswordStore : public PasswordStoreInterface {
   ActionableError GetError() const override;
   void AddLogin(StoredCredential form,
                 base::OnceClosure completion = base::DoNothing()) override;
+  void AddLoginWithResult(
+      StoredCredential form,
+      base::OnceCallback<void(PasswordStoreAddStatus)> completion) override;
+  void GetAllLoginsStrict(
+      base::WeakPtr<PasswordStoreConsumer> consumer) override;
+  void GetLoginsStrict(const PasswordFormDigest& form,
+                       base::WeakPtr<PasswordStoreConsumer> consumer) override;
   void AddLogins(std::vector<StoredCredential> forms,
                  base::OnceClosure completion = base::DoNothing()) override;
   void UpdateLogin(StoredCredential form,
