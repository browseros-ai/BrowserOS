//! Primitive stored rows, independent of domain parsing. Only TraceBatch writes them; the
//! baseline migration owns their schema. The derive macro requires public Model/Relation types;
//! these crate-private modules keep them out of the public trace API. No database foreign keys
//! constrain orphan recovery.

pub(crate) mod operations;
pub(crate) mod private_tabs;
pub(crate) mod run_tabs;
pub(crate) mod runs;
