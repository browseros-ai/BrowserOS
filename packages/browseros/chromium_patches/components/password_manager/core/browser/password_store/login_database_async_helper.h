diff --git a/components/password_manager/core/browser/password_store/login_database_async_helper.h b/components/password_manager/core/browser/password_store/login_database_async_helper.h
index 28780894b98afcdbd7cc33f58b0924f10a6724e8..fe87dd526438c5aaffa7efb0c46abaade915f1e5 100644
--- a/components/password_manager/core/browser/password_store/login_database_async_helper.h
+++ b/components/password_manager/core/browser/password_store/login_database_async_helper.h
@@ -69,18 +69,22 @@ class LoginDatabaseAsyncHelper : public PasswordStoreSync {
           on_undecryptable_passwords_removed,
       scoped_refptr<os_crypt_async::Encryptor> encryptor);
 
-  // Synchronous implementation of PasswordStoreBackend interface.
+  // Synchronous implementation of PasswordStoreBackend interface. strict is
+  // carried only by automation reads that must prove inventory completeness.
   base::expected<std::vector<StoredCredential>, PasswordStoreBackendError>
-  GetAllLogins();
+  GetAllLogins(bool strict = false);
   base::expected<std::vector<StoredCredential>, PasswordStoreBackendError>
   GetAutofillableLogins();
   base::expected<std::vector<StoredCredential>, PasswordStoreBackendError>
   FillMatchingLogins(const std::vector<PasswordFormDigest>& forms,
-                     bool include_psl);
+                     bool include_psl,
+                     bool strict = false);
 
   base::expected<std::optional<PasswordStoreChangeList>,
                  PasswordStoreBackendError>
   AddLogin(StoredCredential cred);
+  // Transactional insert-only path; never falls back to replacement.
+  PasswordStoreAddResult AddLoginIfAbsent(StoredCredential cred);
   base::expected<std::optional<PasswordStoreChangeList>,
                  PasswordStoreBackendError>
   UpdateLogin(const StoredCredential& cred);
