//! Per-tab stable element reference assignment and resolution across repeated and delta snapshots.
//!
//! Maps each live DOM node's cross-frame [`NodeKey`](crate::tree::NodeKey) to compact canonical
//! snapshot (`e`-ref) and on-demand (`n`-ref) identifiers (`[f<seq>]e<n>` / `[f<seq>]n<n>`),
//! preserving reference stability across snapshots while `(role, name)` stays unchanged and
//! retiring references cleanly on navigation, eviction, or explicit forgetting.

mod tab_ref_table;

pub use tab_ref_table::{
    DEFAULT_MAX_RECORDS, Ref, RefKind, RefLookup, RefParseError, RefTarget, TabRefTable,
};
