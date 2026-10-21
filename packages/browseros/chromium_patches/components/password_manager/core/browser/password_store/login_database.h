diff --git a/components/password_manager/core/browser/password_store/login_database.h b/components/password_manager/core/browser/password_store/login_database.h
index e383dcbf705698f423b85d96e32f527338f3edbf..593faf21fcb2c81fb1cbf3e66158a22c50cfd25c 100644
--- a/components/password_manager/core/browser/password_store/login_database.h
+++ b/components/password_manager/core/browser/password_store/login_database.h
@@ -137,9 +137,12 @@ class LoginDatabase : public EncryptDecryptInterface {
   // and federated credentials.
   // |should_PSL_matching_apply| controls if the PSL matches are included or
   // only the exact matches.
+  // strict rejects partial decryption results; the default preserves the UI's
+  // existing ability to offer other readable passwords.
   [[nodiscard]] bool GetLogins(const PasswordFormDigest& form,
                                bool should_PSL_matching_apply,
-                               std::vector<StoredCredential>* credentials);
+                               std::vector<StoredCredential>* credentials,
+                               bool strict = false);
 
   // Gets all logins created from |begin| onwards (inclusive) and before |end|.
   // You may use a null Time value to do an unbounded search in either
