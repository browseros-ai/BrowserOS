diff --git a/components/password_manager/core/browser/password_store/login_database_async_helper.cc b/components/password_manager/core/browser/password_store/login_database_async_helper.cc
index 919ee98d91cb91572e38e4499d6b11b46cbcd4d7..dfac413178033cd5976ae77320f6fcb04f69627f 100644
--- a/components/password_manager/core/browser/password_store/login_database_async_helper.cc
+++ b/components/password_manager/core/browser/password_store/login_database_async_helper.cc
@@ -105,7 +105,7 @@ bool LoginDatabaseAsyncHelper::Initialize(
 }
 
 base::expected<std::vector<StoredCredential>, PasswordStoreBackendError>
-LoginDatabaseAsyncHelper::GetAllLogins() {
+LoginDatabaseAsyncHelper::GetAllLogins(bool strict) {
   DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
   std::vector<StoredCredential> credentials;
 
@@ -115,7 +115,9 @@ LoginDatabaseAsyncHelper::GetAllLogins() {
   }
   FormRetrievalResult result = login_db_->GetAllLogins(&credentials);
   if (result != FormRetrievalResult::kSuccess &&
-      result != FormRetrievalResult::kEncryptionServiceFailureWithPartialData) {
+      (strict ||
+       result !=
+           FormRetrievalResult::kEncryptionServiceFailureWithPartialData)) {
     return base::unexpected(PasswordStoreBackendError(
         PasswordStoreBackendErrorType::kUncategorized));
   }
@@ -136,13 +138,14 @@ LoginDatabaseAsyncHelper::GetAutofillableLogins() {
 base::expected<std::vector<StoredCredential>, PasswordStoreBackendError>
 LoginDatabaseAsyncHelper::FillMatchingLogins(
     const std::vector<PasswordFormDigest>& forms,
-    bool include_psl) {
+    bool include_psl,
+    bool strict) {
   DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
   std::vector<StoredCredential> results;
   for (const auto& form : forms) {
     std::vector<StoredCredential> matched_credentials;
-    if (!login_db_ ||
-        !login_db_->GetLogins(form, include_psl, &matched_credentials)) {
+    if (!login_db_ || !login_db_->GetLogins(form, include_psl,
+                                            &matched_credentials, strict)) {
 #if BUILDFLAG(IS_MAC) || BUILDFLAG(IS_LINUX)
       return base::unexpected(PasswordStoreBackendError(
           is_encryption_available_
@@ -187,6 +190,50 @@ LoginDatabaseAsyncHelper::AddLogin(StoredCredential cred) {
   return changes;
 }
 
+PasswordStoreAddResult LoginDatabaseAsyncHelper::AddLoginIfAbsent(
+    StoredCredential cred) {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+  auto transaction = CreateTransaction();
+  if (!transaction || !transaction->Begin()) {
+    return {};
+  }
+  // Both native UI/import/sync writes and this request run on the same database
+  // sequence. The SQL transaction also protects against another connection.
+  // Strict reads must not mistake unreadable rows for an absent account.
+  auto existing = GetAllLogins(/*strict=*/true);
+  if (!existing.has_value()) {
+    return {};
+  }
+  bool identical = false;
+  for (const auto& saved : *existing) {
+    if (saved.blocked_by_user || saved.signon_realm != cred.signon_realm ||
+        saved.username_value != cred.username_value) {
+      continue;
+    }
+    if (saved.password_value != cred.password_value) {
+      return {PasswordStoreAddStatus::kConflict, {}};
+    }
+    identical = true;
+  }
+  if (identical) {
+    return {PasswordStoreAddStatus::kUnchanged, {}};
+  }
+  AddCredentialError error = AddCredentialError::kNone;
+  auto changes = AddLoginImpl(std::move(cred), &error);
+  if (error != AddCredentialError::kNone || changes.empty()) {
+    return {};
+  }
+  if (password_sync_bridge_) {
+    password_sync_bridge_->ActOnPasswordStoreChanges(FROM_HERE, changes);
+  }
+  // A change list describes attempted mutations, not a committed write. Sync
+  // metadata and the credential must commit together before kAdded is returned.
+  if (!transaction->Commit()) {
+    return {PasswordStoreAddStatus::kOutcomeUnknown, {}};
+  }
+  return {PasswordStoreAddStatus::kAdded, std::move(changes)};
+}
+
 base::expected<std::optional<PasswordStoreChangeList>,
                PasswordStoreBackendError>
 LoginDatabaseAsyncHelper::UpdateLogin(const StoredCredential& cred) {
