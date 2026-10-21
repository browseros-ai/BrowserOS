diff --git a/components/password_manager/core/browser/password_store/login_database.cc b/components/password_manager/core/browser/password_store/login_database.cc
index 324c3fe13daf34a9c3261af435ffb8e95faac513..45b88fdd4ea12d3ec87704178342c2bcabd676cc 100644
--- a/components/password_manager/core/browser/password_store/login_database.cc
+++ b/components/password_manager/core/browser/password_store/login_database.cc
@@ -1777,7 +1777,8 @@ StoredCredential LoginDatabase::GetFormWithoutPasswordFromStatement(
 
 bool LoginDatabase::GetLogins(const PasswordFormDigest& form,
                               bool should_PSL_matching_apply,
-                              std::vector<StoredCredential>* credentials) {
+                              std::vector<StoredCredential>* credentials,
+                              bool strict) {
   TRACE_EVENT0("passwords", "LoginDatabase::GetLogins");
   CHECK(credentials);
   credentials->clear();
@@ -1822,7 +1823,9 @@ bool LoginDatabase::GetLogins(const PasswordFormDigest& form,
       &s, should_PSL_matching_apply || should_federated_apply ? &form : nullptr,
       credentials);
   if (result != FormRetrievalResult::kSuccess &&
-      result != FormRetrievalResult::kEncryptionServiceFailureWithPartialData) {
+      (strict ||
+       result !=
+           FormRetrievalResult::kEncryptionServiceFailureWithPartialData)) {
     credentials->clear();
     return false;
   }
