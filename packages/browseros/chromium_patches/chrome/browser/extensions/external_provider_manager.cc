diff --git a/chrome/browser/extensions/external_provider_manager.cc b/chrome/browser/extensions/external_provider_manager.cc
index 02b5f23cb062594d555990a29fc81b51ef6170bc..505b4eb29d9adce6dada808be288e4aedd03d70a 100644
--- a/chrome/browser/extensions/external_provider_manager.cc
+++ b/chrome/browser/extensions/external_provider_manager.cc
@@ -291,6 +291,7 @@ bool ExternalProviderManager::OnExternalExtensionFileFound(
   installer->set_expected_version(info.version,
                                   true /* fail_install_if_unexpected */);
   installer->set_install_immediately(info.install_immediately);
+  installer->set_external_install_priority(info.install_priority);
   installer->set_creation_flags(info.creation_flags);
 
   CRXFileInfo file_info(
