diff --git a/components/password_manager/core/browser/password_store/get_logins_with_affiliations_request_handler.h b/components/password_manager/core/browser/password_store/get_logins_with_affiliations_request_handler.h
index 4c530b49f8928a6304605afdaf86b1cc29be4f97..daab9fc703981f5b963d04e26905448b7e0a14eb 100644
--- a/components/password_manager/core/browser/password_store/get_logins_with_affiliations_request_handler.h
+++ b/components/password_manager/core/browser/password_store/get_logins_with_affiliations_request_handler.h
@@ -14,12 +14,13 @@ class AffiliatedMatchHelper;
 
 // Helper function which is used to obtain PasswordForms for a particular login
 // and affiliated logins in parallel. 'callback' is invoked after the operation
-// is finished.
+// is finished. strict is preserved through every parallel store-query branch.
 void GetLoginsWithAffiliationsRequestHandler(
     PasswordFormDigest form,
     PasswordStoreBackend* backend,
     AffiliatedMatchHelper* affiliated_match_helper,
-    LoginsOrErrorReply callback);
+    LoginsOrErrorReply callback,
+    bool strict = false);
 
 }  // namespace password_manager
 
