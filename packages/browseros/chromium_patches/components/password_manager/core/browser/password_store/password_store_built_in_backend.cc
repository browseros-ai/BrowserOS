diff --git a/components/password_manager/core/browser/password_store/password_store_built_in_backend.cc b/components/password_manager/core/browser/password_store/password_store_built_in_backend.cc
index 2aa2dfb47f73ce9ce13a9a809b58b454f0d62c8e..cfec920f66dbfa99e904d218bac2b5c636c7dad0 100644
--- a/components/password_manager/core/browser/password_store/password_store_built_in_backend.cc
+++ b/components/password_manager/core/browser/password_store/password_store_built_in_backend.cc
@@ -284,15 +284,56 @@ void PasswordStoreBuiltInBackend::GetAllLoginsAsync(
   DCHECK(helper_);
   background_task_runner_->PostTaskAndReplyWithResult(
       FROM_HERE,
-      base::BindOnce(
-          &LoginDatabaseAsyncHelper::GetAllLogins,
-          base::Unretained(helper_.get())),  // Safe until `Shutdown()`.
+      base::BindOnce(&LoginDatabaseAsyncHelper::GetAllLogins,
+                     base::Unretained(helper_.get()),
+                     false),  // Safe until `Shutdown()`.
       ReportMetricsForResultCallback<base::expected<
           std::vector<StoredCredential>, PasswordStoreBackendError>>(
           MethodName("GetAllLoginsAsync"))
           .Then(std::move(callback)));
 }
 
+// As in the ordinary methods, helper_ outlives queued work: Shutdown deletes it
+// on this same background sequence after these tasks, then releases the
+// backend.
+void PasswordStoreBuiltInBackend::GetAllLoginsStrictAsync(
+    BackendLoginsOrErrorReply callback) {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+  background_task_runner_->PostTaskAndReplyWithResult(
+      FROM_HERE,
+      base::BindOnce(&LoginDatabaseAsyncHelper::GetAllLogins,
+                     base::Unretained(helper_.get()), true),
+      std::move(callback));
+}
+void PasswordStoreBuiltInBackend::FillMatchingLoginsStrictAsync(
+    BackendLoginsOrErrorReply callback,
+    bool include_psl,
+    const std::vector<PasswordFormDigest>& forms) {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+  background_task_runner_->PostTaskAndReplyWithResult(
+      FROM_HERE,
+      base::BindOnce(&LoginDatabaseAsyncHelper::FillMatchingLogins,
+                     base::Unretained(helper_.get()), forms, include_psl, true),
+      std::move(callback));
+}
+void PasswordStoreBuiltInBackend::GetGroupedMatchingLoginsStrictAsync(
+    const PasswordFormDigest& form,
+    BackendLoginsOrErrorReply callback) {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+  GetLoginsWithAffiliationsRequestHandler(
+      form, this, affiliated_match_helper_.get(), std::move(callback), true);
+}
+void PasswordStoreBuiltInBackend::AddLoginIfAbsentAsync(
+    StoredCredential cred,
+    base::OnceCallback<void(PasswordStoreAddResult)> callback) {
+  DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
+  background_task_runner_->PostTaskAndReplyWithResult(
+      FROM_HERE,
+      base::BindOnce(&LoginDatabaseAsyncHelper::AddLoginIfAbsent,
+                     base::Unretained(helper_.get()), std::move(cred)),
+      std::move(callback));
+}
+
 void PasswordStoreBuiltInBackend::GetAllLoginsWithAffiliationAndBrandingAsync(
     BackendLoginsOrErrorReply callback) {
   DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
@@ -334,7 +375,7 @@ void PasswordStoreBuiltInBackend::FillMatchingLoginsAsync(
       base::BindOnce(
           &LoginDatabaseAsyncHelper::FillMatchingLogins,
           base::Unretained(helper_.get()),  // Safe until `Shutdown()`.
-          forms, include_psl),
+          forms, include_psl, false),
       ReportMetricsForResultCallback<base::expected<
           std::vector<StoredCredential>, PasswordStoreBackendError>>(
           MethodName("FillMatchingLoginsAsync"))
@@ -591,11 +632,10 @@ void PasswordStoreBuiltInBackend::OnEncryptorReceived(
               weak_ptr_factory_.GetWeakPtr()))
           .Then(std::move(remote_form_changes_received));
 
-  auto on_undecryptable_passwords_removed =
-      base::BindPostTaskToCurrentDefault(base::BindRepeating(
-          &PasswordStoreBuiltInBackend::
-              SetClearingUndecryptablePasswordsIsEnabledPref,
-          weak_ptr_factory_.GetWeakPtr()));
+  auto on_undecryptable_passwords_removed = base::BindPostTaskToCurrentDefault(
+      base::BindRepeating(&PasswordStoreBuiltInBackend::
+                              SetClearingUndecryptablePasswordsIsEnabledPref,
+                          weak_ptr_factory_.GetWeakPtr()));
 
   background_task_runner_->PostTaskAndReplyWithResult(
       FROM_HERE,
