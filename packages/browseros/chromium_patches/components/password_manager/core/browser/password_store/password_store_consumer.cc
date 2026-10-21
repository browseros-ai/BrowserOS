diff --git a/components/password_manager/core/browser/password_store/password_store_consumer.cc b/components/password_manager/core/browser/password_store/password_store_consumer.cc
index 7af7551b18a6eae1709390e573a62b6dceed19ea..fe7b736a160a04d9fb3a437b01343477f84a9575 100644
--- a/components/password_manager/core/browser/password_store/password_store_consumer.cc
+++ b/components/password_manager/core/browser/password_store/password_store_consumer.cc
@@ -11,6 +11,27 @@
 
 namespace password_manager {
 
+void PasswordStoreInterface::AddLoginWithResult(
+    StoredCredential form,
+    base::OnceCallback<void(PasswordStoreAddStatus)> completion) {
+  std::move(completion).Run(PasswordStoreAddStatus::kError);
+}
+
+void PasswordStoreInterface::GetAllLoginsStrict(
+    base::WeakPtr<PasswordStoreConsumer> consumer) {
+  if (consumer) {
+    consumer->OnGetPasswordStoreResultsOrErrorFrom(
+        this, base::unexpected(PasswordStoreBackendError(
+                  PasswordStoreBackendErrorType::kUncategorized)));
+  }
+}
+
+void PasswordStoreInterface::GetLoginsStrict(
+    const PasswordFormDigest& form,
+    base::WeakPtr<PasswordStoreConsumer> consumer) {
+  GetAllLoginsStrict(consumer);
+}
+
 PasswordStoreConsumer::PasswordStoreConsumer() = default;
 
 PasswordStoreConsumer::~PasswordStoreConsumer() = default;
