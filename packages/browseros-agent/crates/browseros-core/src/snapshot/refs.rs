use crate::{FrameId, Ref};
use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
};

/// Document identity synthesized for snapshot refs as `frame_id:loader_id`, not a raw CDP field.
/// A change starts fresh stable-ref assignment for that document scope.
pub type DocumentId = String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefEntry {
    pub ref_id: Ref,
    pub backend_node_id: i64,
    pub role: String,
    pub name: String,
    /// Zero-based occurrence among nodes with the same frame, role, and name in the latest capture.
    /// Recomputed per snapshot and used only to recover a ref whose backend node id went stale.
    pub nth: usize,
    pub frame_id: Option<FrameId>,
}

#[cfg(test)]
mod tests {
    use super::{MintRef, RefMap};
    use crate::{FrameId, Ref};

    #[test]
    fn reuses_refs_for_same_document_backend_node_across_snapshots() {
        let mut refs = RefMap::new();
        let first = refs.mint(MintRef {
            document_id: Some("main:loader-1"),
            backend_node_id: 1,
            role: "button",
            name: "A",
            frame_id: None,
        });
        let second = refs.mint(MintRef {
            document_id: Some("main:loader-1"),
            backend_node_id: 2,
            role: "link",
            name: "B",
            frame_id: None,
        });

        refs.begin_snapshot();
        let inserted = refs.mint(MintRef {
            document_id: Some("main:loader-1"),
            backend_node_id: 3,
            role: "button",
            name: "X",
            frame_id: None,
        });
        let first_again = refs.mint(MintRef {
            document_id: Some("main:loader-1"),
            backend_node_id: 1,
            role: "button",
            name: "A",
            frame_id: None,
        });
        let second_again = refs.mint(MintRef {
            document_id: Some("main:loader-1"),
            backend_node_id: 2,
            role: "link",
            name: "B",
            frame_id: None,
        });

        assert_eq!(first_again, first);
        assert_eq!(second_again, second);
        assert_eq!(inserted, Ref("e3".to_string()));
        let ordered = refs
            .entries_in_order()
            .into_iter()
            .map(|entry| entry.ref_id.as_str().to_string())
            .collect::<Vec<_>>();
        assert_eq!(ordered, vec!["e3", "e1", "e2"]);
    }

    #[test]
    fn keeps_latest_snapshot_refs_while_preserving_stable_assignments() {
        let mut refs = RefMap::new();
        refs.mint(MintRef {
            document_id: Some("main:loader-1"),
            backend_node_id: 1,
            role: "button",
            name: "A",
            frame_id: None,
        });
        refs.mint(MintRef {
            document_id: Some("main:loader-1"),
            backend_node_id: 2,
            role: "button",
            name: "B",
            frame_id: None,
        });

        refs.begin_snapshot();
        let kept = refs.mint(MintRef {
            document_id: Some("main:loader-1"),
            backend_node_id: 2,
            role: "button",
            name: "B",
            frame_id: None,
        });

        assert_eq!(kept, Ref("e2".to_string()));
        assert!(refs.get(&Ref("e1".to_string())).is_none());
        assert_eq!(
            refs.get(&Ref("e2".to_string()))
                .map(|entry| entry.backend_node_id),
            Some(2)
        );
    }

    #[test]
    fn resets_public_namespace_for_new_document() {
        let mut refs = RefMap::new();
        assert_eq!(
            refs.mint(MintRef {
                document_id: Some("main:loader-1"),
                backend_node_id: 10,
                role: "button",
                name: "Old",
                frame_id: None,
            }),
            Ref("e1".to_string())
        );
        refs.mint(MintRef {
            document_id: Some("main:loader-1"),
            backend_node_id: 11,
            role: "button",
            name: "Second",
            frame_id: None,
        });

        refs.reset();

        assert_eq!(
            refs.mint(MintRef {
                document_id: Some("main:loader-2"),
                backend_node_id: 20,
                role: "button",
                name: "New",
                frame_id: None,
            }),
            Ref("e1".to_string())
        );
        assert_eq!(refs.len(), 1);
    }

    #[test]
    fn uses_capture_local_traversal_order_without_document_identity() {
        let mut refs = RefMap::new();
        assert_eq!(
            refs.mint(MintRef {
                backend_node_id: 1,
                role: "button",
                name: "Fallback",
                document_id: None,
                frame_id: None,
            }),
            Ref("e1".to_string())
        );
        refs.begin_snapshot();
        assert_eq!(
            refs.mint(MintRef {
                backend_node_id: 2,
                role: "button",
                name: "Fallback",
                document_id: None,
                frame_id: None,
            }),
            Ref("e1".to_string())
        );
    }

    #[test]
    fn scopes_backend_node_identity_by_frame_document() {
        let mut refs = RefMap::new();
        let child_frame = FrameId("child".to_string());
        let main = refs.mint(MintRef {
            document_id: Some("main:loader-1"),
            backend_node_id: 7,
            role: "button",
            name: "Submit",
            frame_id: None,
        });
        let child = refs.mint(MintRef {
            document_id: Some("child:loader-1"),
            backend_node_id: 7,
            role: "button",
            name: "Submit",
            frame_id: Some(&child_frame),
        });

        refs.begin_snapshot();
        assert_eq!(
            refs.mint(MintRef {
                document_id: Some("main:loader-1"),
                backend_node_id: 7,
                role: "button",
                name: "Submit",
                frame_id: None,
            }),
            main
        );
        assert_eq!(
            refs.mint(MintRef {
                document_id: Some("child:loader-1"),
                backend_node_id: 7,
                role: "button",
                name: "Submit",
                frame_id: Some(&child_frame),
            }),
            child
        );
        assert_ne!(main, child);
    }

    #[test]
    fn recomputes_duplicate_nth_metadata_for_each_snapshot() {
        let mut refs = RefMap::new();
        refs.mint(MintRef {
            document_id: Some("main:loader-1"),
            backend_node_id: 1,
            role: "button",
            name: "OK",
            frame_id: None,
        });
        refs.mint(MintRef {
            document_id: Some("main:loader-1"),
            backend_node_id: 2,
            role: "button",
            name: "OK",
            frame_id: None,
        });
        refs.begin_snapshot();
        let second = refs.mint(MintRef {
            document_id: Some("main:loader-1"),
            backend_node_id: 2,
            role: "button",
            name: "OK",
            frame_id: None,
        });
        let first = refs.mint(MintRef {
            document_id: Some("main:loader-1"),
            backend_node_id: 1,
            role: "button",
            name: "OK",
            frame_id: None,
        });
        assert_eq!(refs.get(&second).map(|entry| entry.nth), Some(0));
        assert_eq!(refs.get(&first).map(|entry| entry.nth), Some(1));
    }
}

#[derive(Debug, Clone)]
pub struct RefMap {
    by_ref: HashMap<Ref, RefEntry>,
    order: Vec<Ref>,
    next_ref_num: u32,
    next_fallback_ref_num: u32,
    by_stable_node: HashMap<StableNodeKey, Ref>,
    stable_refs: HashSet<Ref>,
    nth_counter: HashMap<NthKey, usize>,
}

impl Default for RefMap {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Eq)]
struct StableNodeKey {
    document_id: DocumentId,
    frame_id: Option<FrameId>,
    backend_node_id: i64,
}

impl PartialEq for StableNodeKey {
    fn eq(&self, other: &Self) -> bool {
        self.document_id == other.document_id
            && self.frame_id == other.frame_id
            && self.backend_node_id == other.backend_node_id
    }
}

impl Hash for StableNodeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.document_id.hash(state);
        self.frame_id.hash(state);
        self.backend_node_id.hash(state);
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
struct NthKey {
    frame_id: Option<FrameId>,
    role: String,
    name: String,
}

pub struct MintRef<'a> {
    pub backend_node_id: i64,
    pub role: &'a str,
    pub name: &'a str,
    pub document_id: Option<&'a str>,
    pub frame_id: Option<&'a FrameId>,
}

impl RefMap {
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_ref_num: 1,
            next_fallback_ref_num: 1,
            by_ref: HashMap::new(),
            order: Vec::new(),
            by_stable_node: HashMap::new(),
            stable_refs: HashSet::new(),
            nth_counter: HashMap::new(),
        }
    }

    pub fn begin_snapshot(&mut self) {
        self.by_ref.clear();
        self.order.clear();
        self.nth_counter.clear();
        self.next_fallback_ref_num = 1;
    }

    #[must_use]
    pub fn fork_for_snapshot(&self) -> Self {
        let mut fork = Self::new();
        fork.next_ref_num = self.next_ref_num;
        fork.by_stable_node = self.by_stable_node.clone();
        fork.stable_refs = self.stable_refs.clone();
        fork.begin_snapshot();
        fork
    }

    pub fn reset(&mut self) {
        self.by_ref.clear();
        self.order.clear();
        self.by_stable_node.clear();
        self.stable_refs.clear();
        self.nth_counter.clear();
        self.next_ref_num = 1;
        self.next_fallback_ref_num = 1;
    }

    pub fn mint(&mut self, node: MintRef<'_>) -> Ref {
        let nth_key = NthKey {
            frame_id: node.frame_id.cloned(),
            role: node.role.to_string(),
            name: node.name.to_string(),
        };
        let nth = *self.nth_counter.get(&nth_key).unwrap_or(&0);
        self.nth_counter.insert(nth_key, nth + 1);

        let stable_key = node.document_id.map(|document_id| StableNodeKey {
            document_id: document_id.to_string(),
            frame_id: node.frame_id.cloned(),
            backend_node_id: node.backend_node_id,
        });
        let ref_id = match stable_key {
            Some(key) => self.ref_for_stable_node(key),
            None => self.next_fallback_ref(),
        };

        let entry = RefEntry {
            ref_id: ref_id.clone(),
            backend_node_id: node.backend_node_id,
            role: node.role.to_string(),
            name: node.name.to_string(),
            nth,
            frame_id: node.frame_id.cloned(),
        };
        if !self.by_ref.contains_key(&ref_id) {
            self.order.push(ref_id.clone());
        }
        self.by_ref.insert(ref_id.clone(), entry);
        ref_id
    }

    #[must_use]
    pub fn get(&self, ref_id: &Ref) -> Option<&RefEntry> {
        self.by_ref.get(ref_id)
    }

    pub fn get_mut(&mut self, ref_id: &Ref) -> Option<&mut RefEntry> {
        self.by_ref.get_mut(ref_id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_ref.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_ref.is_empty()
    }

    #[must_use]
    pub fn entries_in_order(&self) -> Vec<&RefEntry> {
        self.order
            .iter()
            .filter_map(|ref_id| self.by_ref.get(ref_id))
            .collect()
    }

    fn ref_for_stable_node(&mut self, key: StableNodeKey) -> Ref {
        if let Some(existing) = self.by_stable_node.get(&key) {
            return existing.clone();
        }
        let ref_id = self.next_ref();
        self.by_stable_node.insert(key, ref_id.clone());
        self.stable_refs.insert(ref_id.clone());
        ref_id
    }

    fn next_ref(&mut self) -> Ref {
        loop {
            let ref_id = Ref(format!("e{}", self.next_ref_num));
            self.next_ref_num += 1;
            if !self.is_reserved(&ref_id) {
                return ref_id;
            }
        }
    }

    fn next_fallback_ref(&mut self) -> Ref {
        loop {
            let ref_id = Ref(format!("e{}", self.next_fallback_ref_num));
            self.next_fallback_ref_num += 1;
            if !self.is_reserved(&ref_id) {
                return ref_id;
            }
        }
    }

    fn is_reserved(&self, ref_id: &Ref) -> bool {
        self.by_ref.contains_key(ref_id) || self.stable_refs.contains(ref_id)
    }
}
