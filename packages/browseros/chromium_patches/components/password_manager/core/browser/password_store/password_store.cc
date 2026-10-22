diff --git a/components/password_manager/core/browser/password_store/password_store.cc b/components/password_manager/core/browser/password_store/password_store.cc
index f3742142740aa487d7a7d7b0e6855c46880d79b0..1a2ef813be897bcbae0108d62e84e11424916e24 100644
--- a/components/password_manager/core/browser/password_store/password_store.cc
+++ b/components/password_manager/core/browser/password_store/password_store.cc
@@ -99,6 +99,100 @@ void PasswordStore::AddLogin(StoredCredential form,
   AddLogins(std::move(forms), std::move(completion));
 }
 
+// Default backend implementations preserve old platform implementations while
+// refusing guarantees they cannot provide. Desktop overrides execute on its
+// database sequence; no fallback is allowed to the ordinary replacing add.
+void PasswordStoreBackend::GetAllLoginsStrictAsync(
+    BackendLoginsOrErrorReply callback) {
+  std::move(callback).Run(base::unexpected(PasswordStoreBackendError(
+      PasswordStoreBackendErrorType::kUncategorized)));
+}
+void PasswordStoreBackend::FillMatchingLoginsStrictAsync(
+    BackendLoginsOrErrorReply callback,
+    bool include_psl,
+    const std::vector<PasswordFormDigest>& forms) {
+  GetAllLoginsStrictAsync(std::move(callback));
+}
+void PasswordStoreBackend::GetGroupedMatchingLoginsStrictAsync(
+    const PasswordFormDigest& form,
+    BackendLoginsOrErrorReply callback) {
+  GetAllLoginsStrictAsync(std::move(callback));
+}
+void PasswordStoreBackend::AddLoginIfAbsentAsync(
+    StoredCredential cred,
+    base::OnceCallback<void(PasswordStoreAddResult)> callback) {
+  std::move(callback).Run(PasswordStoreAddResult{});
+}
+
+void PasswordStore::AddLoginWithResult(
+    StoredCredential form,
+    base::OnceCallback<void(PasswordStoreAddStatus)> completion) {
+  DCHECK(main_task_runner_->RunsTasksInCurrentSequence());
+  if (!backend_) {
+    std::move(completion).Run(PasswordStoreAddStatus::kError);
+    return;
+  }
+  if (post_init_callback_) {
+    post_init_callback_ =
+        std::move(post_init_callback_)
+            .Then(base::BindOnce(&PasswordStore::AddLoginWithResult, this,
+                                 std::move(form), std::move(completion)));
+    return;
+  }
+  // The backend checks and inserts atomically with ordinary native/sync writes.
+  // Notify the usual observers only after a confirmed commit, before replying.
+  backend_->AddLoginIfAbsentAsync(
+      std::move(form),
+      base::BindOnce(
+          [](scoped_refptr<PasswordStore> store,
+             base::OnceCallback<void(PasswordStoreAddStatus)> done,
+             PasswordStoreAddResult result) {
+            if (result.status == PasswordStoreAddStatus::kAdded) {
+              store->NotifyLoginsChangedOnMainSequence(
+                  LoginsChangedTrigger::Addition, std::move(result.changes));
+            }
+            std::move(done).Run(result.status);
+          },
+          base::WrapRefCounted(this), std::move(completion)));
+}
+
+void PasswordStore::GetAllLoginsStrict(
+    base::WeakPtr<PasswordStoreConsumer> consumer) {
+  DCHECK(main_task_runner_->RunsTasksInCurrentSequence());
+  if (!backend_) {
+    PasswordStoreInterface::GetAllLoginsStrict(consumer);
+    return;
+  }
+  if (post_init_callback_) {
+    post_init_callback_ =
+        std::move(post_init_callback_)
+            .Then(base::BindOnce(&PasswordStore::GetAllLoginsStrict, this,
+                                 consumer));
+    return;
+  }
+  backend_->GetAllLoginsStrictAsync(base::BindOnce(
+      &PasswordStore::ForwardLoginsResultOrError, this, consumer));
+}
+void PasswordStore::GetLoginsStrict(
+    const PasswordFormDigest& form,
+    base::WeakPtr<PasswordStoreConsumer> consumer) {
+  DCHECK(main_task_runner_->RunsTasksInCurrentSequence());
+  if (!backend_) {
+    PasswordStoreInterface::GetAllLoginsStrict(consumer);
+    return;
+  }
+  if (post_init_callback_) {
+    post_init_callback_ =
+        std::move(post_init_callback_)
+            .Then(base::BindOnce(&PasswordStore::GetLoginsStrict, this, form,
+                                 consumer));
+    return;
+  }
+  backend_->GetGroupedMatchingLoginsStrictAsync(
+      form, base::BindOnce(&PasswordStore::ForwardLoginsResultOrError, this,
+                           consumer));
+}
+
 void PasswordStore::AddLogins(std::vector<StoredCredential> forms,
                               base::OnceClosure completion) {
   DCHECK(main_task_runner_->RunsTasksInCurrentSequence());
