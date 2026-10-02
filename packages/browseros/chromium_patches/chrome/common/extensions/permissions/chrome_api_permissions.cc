diff --git a/chrome/common/extensions/permissions/chrome_api_permissions.cc b/chrome/common/extensions/permissions/chrome_api_permissions.cc
index 86f4a132b4d8c2d517af0b5bed9a40dfa216b841..6d4d20b7cdcc8d9a3f8865b94a5931a73cbe3763 100644
--- a/chrome/common/extensions/permissions/chrome_api_permissions.cc
+++ b/chrome/common/extensions/permissions/chrome_api_permissions.cc
@@ -67,6 +67,7 @@ constexpr APIPermissionInfo::InitInfo permissions_to_register[] = {
     {APIPermissionID::kBookmark, "bookmarks"},
     {APIPermissionID::kBrailleDisplayPrivate, "brailleDisplayPrivate",
      APIPermissionInfo::kFlagCannotBeOptional},
+    {APIPermissionID::kBrowserOS, "browserOS"},
     {APIPermissionID::kBrowsingData, "browsingData",
      APIPermissionInfo::kFlagDoesNotRequireManagedSessionFullLoginWarning},
     {APIPermissionID::kCertificateProvider, "certificateProvider",
