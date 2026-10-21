diff --git a/components/password_manager/core/browser/password_store/password_store_interface.h b/components/password_manager/core/browser/password_store/password_store_interface.h
index 4e8f2af809762cdaf82adfdf26d489781b977252..35a3874227a14232e6b4c6ade91bc672a026fc48 100644
--- a/components/password_manager/core/browser/password_store/password_store_interface.h
+++ b/components/password_manager/core/browser/password_store/password_store_interface.h
@@ -5,6 +5,7 @@
 #ifndef COMPONENTS_PASSWORD_MANAGER_CORE_BROWSER_PASSWORD_STORE_PASSWORD_STORE_INTERFACE_H_
 #define COMPONENTS_PASSWORD_MANAGER_CORE_BROWSER_PASSWORD_STORE_PASSWORD_STORE_INTERFACE_H_
 
+#include <optional>
 #include <vector>
 
 #include "base/functional/callback_helpers.h"
@@ -90,6 +91,20 @@ class PasswordStoreInterface : public RefcountedKeyedService {
   virtual void AddLogin(StoredCredential form,
                         base::OnceClosure completion = base::DoNothing()) = 0;
 
+  // Atomic insert-only add: conflict checking and commit run on the storage
+  // sequence. Unlike AddLogin's closure this distinguishes a confirmed commit,
+  // unchanged data, conflicts and unknown outcomes. Unsupported stores refuse.
+  virtual void AddLoginWithResult(
+      StoredCredential form,
+      base::OnceCallback<void(PasswordStoreAddStatus)> completion);
+
+  // Strict inventory preserves partial/decryption failures instead of silently
+  // omitting unreadable rows. Automation uses it for complete lists/preflight.
+  virtual void GetAllLoginsStrict(
+      base::WeakPtr<PasswordStoreConsumer> consumer);
+  virtual void GetLoginsStrict(const PasswordFormDigest& form,
+                               base::WeakPtr<PasswordStoreConsumer> consumer);
+
   // Adds all forms in the given vector of StoredCredential to the secure
   // password store asynchronously. `completion` will be run after the forms are
   // added.
