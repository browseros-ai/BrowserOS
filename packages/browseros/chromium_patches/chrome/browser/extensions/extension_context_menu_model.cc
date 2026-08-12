diff --git a/chrome/browser/extensions/extension_context_menu_model.cc b/chrome/browser/extensions/extension_context_menu_model.cc
index 94417bcccd3ae41a9c9e781336e595be27c2391d..ab50329ef8988a9b6506a76cf8583575729968fb 100644
--- a/chrome/browser/extensions/extension_context_menu_model.cc
+++ b/chrome/browser/extensions/extension_context_menu_model.cc
@@ -15,6 +15,7 @@
 #include "base/notreached.h"
 #include "base/strings/utf_string_conversions.h"
 #include "base/types/pass_key.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/extensions/context_menu_matcher.h"
 #include "chrome/browser/extensions/extension_management.h"
 #include "chrome/browser/extensions/extension_tab_util.h"
@@ -93,13 +94,15 @@ namespace {
 // Returns true if the given |item| is of the given |type|.
 bool MenuItemMatchesAction(const std::optional<ActionInfo::Type> action_type,
                            const MenuItem* item) {
-  if (!action_type)
+  if (!action_type) {
     return false;
+  }
 
   const MenuItem::ContextList& contexts = item->contexts();
 
-  if (contexts.Contains(MenuItem::ALL))
+  if (contexts.Contains(MenuItem::ALL)) {
     return true;
+  }
   if (contexts.Contains(MenuItem::PAGE_ACTION) &&
       (*action_type == ActionInfo::Type::kPage)) {
     return true;
@@ -391,11 +394,13 @@ void ExtensionContextMenuModel::Init(const Extension* extension,
 
 bool ExtensionContextMenuModel::IsCommandIdChecked(int command_id) const {
   const Extension* extension = GetExtension();
-  if (!extension)
+  if (!extension) {
     return false;
+  }
 
-  if (ContextMenuMatcher::IsExtensionsCustomCommandId(command_id))
+  if (ContextMenuMatcher::IsExtensionsCustomCommandId(command_id)) {
     return extension_items_->IsCommandIdChecked(command_id);
+  }
 
   if (command_id == PAGE_ACCESS_RUN_ON_CLICK ||
       command_id == PAGE_ACCESS_RUN_ON_SITE ||
@@ -415,11 +420,13 @@ bool ExtensionContextMenuModel::IsCommandIdChecked(int command_id) const {
 
 bool ExtensionContextMenuModel::IsCommandIdVisible(int command_id) const {
   const Extension* extension = GetExtension();
-  if (!extension)
+  if (!extension) {
     return false;
+  }
 
-  if (ContextMenuMatcher::IsExtensionsCustomCommandId(command_id))
+  if (ContextMenuMatcher::IsExtensionsCustomCommandId(command_id)) {
     return extension_items_->IsCommandIdVisible(command_id);
+  }
 
   // Items added by Chrome to the menu are always visible.
   return true;
@@ -427,11 +434,13 @@ bool ExtensionContextMenuModel::IsCommandIdVisible(int command_id) const {
 
 bool ExtensionContextMenuModel::IsCommandIdEnabled(int command_id) const {
   const Extension* extension = GetExtension();
-  if (!extension)
+  if (!extension) {
     return false;
+  }
 
-  if (ContextMenuMatcher::IsExtensionsCustomCommandId(command_id))
+  if (ContextMenuMatcher::IsExtensionsCustomCommandId(command_id)) {
     return extension_items_->IsCommandIdEnabled(command_id);
+  }
 
   switch (command_id) {
     case HOME_PAGE:
@@ -507,8 +516,9 @@ void ExtensionContextMenuModel::RecordUkmForExtension(
 void ExtensionContextMenuModel::ExecuteCommand(int command_id,
                                                int event_flags) {
   const Extension* extension = GetExtension();
-  if (!extension)
+  if (!extension) {
     return;
+  }
 
   if (ContextMenuMatcher::IsExtensionsCustomCommandId(command_id)) {
     DCHECK(extension_items_);
@@ -838,7 +848,9 @@ void ExtensionContextMenuModel::InitMenuWithFeature(
 
   // Controls section.
   bool has_options_page = OptionsPageInfo::HasOptionsPage(extension);
-  bool can_uninstall_extension = !is_component_ && !is_required_by_policy;
+  bool can_uninstall_extension =
+      !is_component_ && !is_required_by_policy &&
+      !browseros::IsActiveBrowserOSExtension(extension->id());
   if (can_show_icon_in_toolbar || has_options_page || can_uninstall_extension) {
     AddSeparator(ui::NORMAL_SEPARATOR);
   }
@@ -906,8 +918,9 @@ void ExtensionContextMenuModel::InitMenu(const Extension* extension,
   std::optional<ActionInfo::Type> action_type;
   extension_action_ =
       ExtensionActionManager::Get(profile_)->GetExtensionAction(*extension);
-  if (extension_action_)
+  if (extension_action_) {
     action_type = extension_action_->action_type();
+  }
 
   extension_items_ = std::make_unique<ContextMenuMatcher>(
       profile_, this, this,
@@ -932,8 +945,9 @@ void ExtensionContextMenuModel::InitMenu(const Extension* extension,
     AddSeparator(ui::NORMAL_SEPARATOR);
   }
 
-  if (OptionsPageInfo::HasOptionsPage(extension))
+  if (OptionsPageInfo::HasOptionsPage(extension)) {
     AddItemWithStringId(OPTIONS, IDS_EXTENSIONS_OPTIONS_MENU_ITEM);
+  }
 
   if (!is_component_) {
     if (IsExtensionRequiredByPolicy(extension, profile_)) {
@@ -1022,8 +1036,9 @@ const Extension* ExtensionContextMenuModel::GetExtension() const {
 void ExtensionContextMenuModel::AppendExtensionItems() {
   MenuManager* menu_manager = MenuManager::Get(profile_);
   if (!menu_manager ||  // Null in unit tests
-      !menu_manager->MenuItems(MenuItem::ExtensionKey(extension_id_)))
+      !menu_manager->MenuItems(MenuItem::ExtensionKey(extension_id_))) {
     return;
+  }
 
   AddSeparator(ui::NORMAL_SEPARATOR);
 
