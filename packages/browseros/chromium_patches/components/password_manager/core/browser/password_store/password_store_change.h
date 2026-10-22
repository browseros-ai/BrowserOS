diff --git a/components/password_manager/core/browser/password_store/password_store_change.h b/components/password_manager/core/browser/password_store/password_store_change.h
index 7486bd8d42a8e794d46281f22dfc1e4413f46439..e73a2b6de73d1daaa20b3f9201f82c1b07ac5752 100644
--- a/components/password_manager/core/browser/password_store/password_store_change.h
+++ b/components/password_manager/core/browser/password_store/password_store_change.h
@@ -54,6 +54,21 @@ class PasswordStoreChange {
 
 using PasswordStoreChangeList = std::vector<PasswordStoreChange>;
 
+// Result of an insert-only credential operation. Added is acknowledged only
+// after the credential and its sync metadata commit in one database
+// transaction.
+enum class PasswordStoreAddStatus {
+  kAdded,
+  kUnchanged,
+  kConflict,
+  kError,
+  kOutcomeUnknown,
+};
+struct PasswordStoreAddResult {
+  PasswordStoreAddStatus status = PasswordStoreAddStatus::kError;
+  PasswordStoreChangeList changes;
+};
+
 // For testing.
 #if defined(UNIT_TEST)
 inline std::ostream& operator<<(
