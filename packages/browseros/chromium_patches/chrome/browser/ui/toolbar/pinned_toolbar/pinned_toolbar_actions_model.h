diff --git a/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.h b/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.h
index a183b49e14fbd5d7a74f01132d24ce13706ead47..994b90982961a3d6ca866c41f2a0d71499a03da6 100644
--- a/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.h
+++ b/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.h
@@ -55,6 +55,9 @@ class PinnedToolbarActionsModel : public KeyedService {
     // the above methods, this does include pref updates.
     virtual void OnActionsChanged() {}
 
+    // Called when toolbar label visibility pref changes.
+    virtual void OnLabelsVisibilityChanged() {}
+
    protected:
     virtual ~Observer() = default;
   };
@@ -95,6 +98,11 @@ class PinnedToolbarActionsModel : public KeyedService {
   // TODO(crbug.com/353323253): Remove after all migrations are complete.
   void MaybeMigrateExistingPinnedStates();
 
+  // Ensures that certain actions are always pinned to the toolbar.
+  // This is called during initialization to ensure specific actions
+  // (like Third Party LLM) are always visible.
+  void EnsureAlwaysPinnedActions();
+
   // Returns the ordered list of pinned ActionIds.
   virtual const std::vector<actions::ActionId>& PinnedActionIds() const;
 
@@ -113,6 +121,14 @@ class PinnedToolbarActionsModel : public KeyedService {
 
   void UpdatePref(const std::vector<actions::ActionId>& updated_list);
 
+  // Called when a BrowserOS visibility pref changes.
+  // Re-evaluates which actions should be pinned and notifies observers.
+  void OnBrowserOSVisibilityPrefChanged();
+
+  // Called when the toolbar labels pref changes.
+  // Notifies observers so buttons can refresh their labels.
+  void OnBrowserOSLabelsPrefChanged();
+
   // Our observers.
   base::ObserverList<Observer>::Unchecked observers_;
 
