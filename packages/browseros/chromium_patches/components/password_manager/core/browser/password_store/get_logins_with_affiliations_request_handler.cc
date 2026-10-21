diff --git a/components/password_manager/core/browser/password_store/get_logins_with_affiliations_request_handler.cc b/components/password_manager/core/browser/password_store/get_logins_with_affiliations_request_handler.cc
index 12b05a580d9c1bcb4059f454ad547e88b261a6f8..7c0b36b802a14e3c88a4bd9ae02e5b2b40b39a69 100644
--- a/components/password_manager/core/browser/password_store/get_logins_with_affiliations_request_handler.cc
+++ b/components/password_manager/core/browser/password_store/get_logins_with_affiliations_request_handler.cc
@@ -117,9 +117,11 @@ void TrimUsernameOnlyCredentials(std::vector<StoredCredential>& credentials) {
 class GetLoginsHelper : public base::RefCounted<GetLoginsHelper> {
  public:
   GetLoginsHelper(PasswordFormDigest requested_digest,
-                  PasswordStoreBackend* backend)
+                  PasswordStoreBackend* backend,
+                  bool strict)
       : requested_digest_(std::move(requested_digest)),
-        backend_(backend->AsWeakPtr()) {}
+        backend_(backend->AsWeakPtr()),
+        strict_(strict) {}
 
   void Init(AffiliatedMatchHelper* affiliated_match_helper,
             LoginsOrErrorReply callback);
@@ -150,6 +152,17 @@ class GetLoginsHelper : public base::RefCounted<GetLoginsHelper> {
   MergeResults(std::vector<base::expected<std::vector<StoredCredential>,
                                           PasswordStoreBackendError>> results);
 
+  void FillMatchingLogins(BackendLoginsOrErrorReply callback,
+                          bool include_psl,
+                          const std::vector<PasswordFormDigest>& forms) {
+    if (strict_) {
+      backend_->FillMatchingLoginsStrictAsync(std::move(callback), include_psl,
+                                              forms);
+    } else {
+      backend_->FillMatchingLoginsAsync(std::move(callback), include_psl,
+                                        forms);
+    }
+  }
   const PasswordFormDigest requested_digest_;
 
   // All the affiliations for 'requested_digest_'.
@@ -161,6 +174,7 @@ class GetLoginsHelper : public base::RefCounted<GetLoginsHelper> {
   GURL change_password_url_;
 
   base::WeakPtr<PasswordStoreBackend> backend_;
+  const bool strict_;
 };
 
 void GetLoginsHelper::Init(AffiliatedMatchHelper* affiliated_match_helper,
@@ -168,7 +182,7 @@ void GetLoginsHelper::Init(AffiliatedMatchHelper* affiliated_match_helper,
   if (!affiliated_match_helper) {
     // If |affiliated_match_helper| is unavailable return only exact and PSL
     // matches.
-    backend_->FillMatchingLoginsAsync(
+    FillMatchingLogins(
         base::BindOnce(&ProcessExactAndPSLForms, requested_digest_,
                        base::flat_set<std::string>())
             .Then(std::move(callback)),
@@ -205,11 +219,10 @@ void GetLoginsHelper::OnPSLExtensionsReceived(
   if (!backend_) {
     return;
   }
-  backend_->FillMatchingLoginsAsync(
-      base::BindOnce(&ProcessExactAndPSLForms, requested_digest_,
-                     psl_extensions)
-          .Then(std::move(forms_received_callback)),
-      FormSupportsPSL(requested_digest_), {requested_digest_});
+  FillMatchingLogins(base::BindOnce(&ProcessExactAndPSLForms, requested_digest_,
+                                    psl_extensions)
+                         .Then(std::move(forms_received_callback)),
+                     FormSupportsPSL(requested_digest_), {requested_digest_});
 }
 
 void GetLoginsHelper::HandleAffiliationsAndGroupsReceived(
@@ -254,8 +267,8 @@ void GetLoginsHelper::HandleAffiliationsAndGroupsReceived(
                                       GURL(realm));
     }
   }
-  backend_->FillMatchingLoginsAsync(forms_received_callback,
-                                    /*include_psl=*/false, digests_to_request);
+  FillMatchingLogins(forms_received_callback,
+                     /*include_psl=*/false, digests_to_request);
 }
 
 base::expected<std::vector<StoredCredential>, PasswordStoreBackendError>
@@ -317,9 +330,10 @@ void GetLoginsWithAffiliationsRequestHandler(
     PasswordFormDigest form,
     PasswordStoreBackend* backend,
     AffiliatedMatchHelper* affiliated_match_helper,
-    LoginsOrErrorReply callback) {
+    LoginsOrErrorReply callback,
+    bool strict) {
   scoped_refptr<GetLoginsHelper> request_handler =
-      base::MakeRefCounted<GetLoginsHelper>(std::move(form), backend);
+      base::MakeRefCounted<GetLoginsHelper>(std::move(form), backend, strict);
   request_handler->Init(affiliated_match_helper, std::move(callback));
 }
 
