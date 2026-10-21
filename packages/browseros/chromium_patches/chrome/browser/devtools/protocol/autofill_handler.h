diff --git a/chrome/browser/devtools/protocol/autofill_handler.h b/chrome/browser/devtools/protocol/autofill_handler.h
index 793a23d6487238cb6bb79371ba7e42181368cc17..9b08da8c0a19bf65a677646817a596fff817f998 100644
--- a/chrome/browser/devtools/protocol/autofill_handler.h
+++ b/chrome/browser/devtools/protocol/autofill_handler.h
@@ -19,8 +19,13 @@ namespace autofill {
 class AutofillClient;
 class ContentAutofillDriver;
 class CreditCard;
-}
+}  // namespace autofill
 
+class CredentialOperation;
+
+// Owns trusted CDP autofill dispatch and its cancellable credential operation.
+// Password-store handles live on Profile, so page-session teardown does not
+// invalidate credentials listed by another session in the same profile.
 class AutofillHandler : public protocol::Autofill::Backend,
                         autofill::AutofillManager::Observer,
                         autofill::ContentAutofillDriverFactory::Observer {
@@ -34,6 +39,32 @@ class AutofillHandler : public protocol::Autofill::Backend,
   ~AutofillHandler() override;
 
  private:
+  protocol::Response GetCredentialContext(
+      std::unique_ptr<protocol::Autofill::CredentialContext>* context) override;
+  void ListCredentials(
+      const String& operation_id,
+      std::optional<String> document_id,
+      std::optional<String> browser_context_id,
+      std::optional<String> query,
+      std::optional<int> limit,
+      std::optional<String> cursor,
+      std::unique_ptr<ListCredentialsCallback> callback) override;
+  void AddCredential(const String& operation_id,
+                     const String& document_id,
+                     const String& username,
+                     const String& password,
+                     std::unique_ptr<AddCredentialCallback> callback) override;
+  void FillCredential(
+      const String& operation_id,
+      const String& document_id,
+      std::optional<String> credential_id,
+      std::optional<String> username,
+      std::optional<String> password,
+      std::unique_ptr<FillCredentialCallback> callback) override;
+  protocol::Response CancelCredentialOperation(
+      const String& operation_id) override;
+  std::unique_ptr<CredentialOperation> credential_operation_;
+
   protocol::Response Enable() override;
   protocol::Response Disable() override;
   void Trigger(int field_id,
