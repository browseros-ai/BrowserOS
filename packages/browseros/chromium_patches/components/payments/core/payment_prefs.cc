diff --git a/components/payments/core/payment_prefs.cc b/components/payments/core/payment_prefs.cc
index d42858bde4cf7..a632291402897 100644
--- a/components/payments/core/payment_prefs.cc
+++ b/components/payments/core/payment_prefs.cc
@@ -11,7 +11,7 @@ namespace payments {
 void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry) {
   registry->RegisterBooleanPref(kPaymentsFirstTransactionCompleted, false);
   registry->RegisterBooleanPref(
-      kCanMakePaymentEnabled, true,
+      kCanMakePaymentEnabled, false,
       user_prefs::PrefRegistrySyncable::SYNCABLE_PREF);
 }
 
