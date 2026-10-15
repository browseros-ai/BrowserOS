//! Who is calling: the client product and agent profile a session resolves to, and the
//! per-conversation key (`{slug}-{fun-name}`) that page ownership is keyed by.

mod client;
mod client_names;
mod conversation;

pub use client::{ClientIdentity, ClientInfo, ProfileView, slugify_client_name};
pub(crate) use client_names::{CLIENT_ALIASES, unversioned_client_slug};
pub use client_names::{canonical_client_name, client_product_prefix};
pub use conversation::{
    ConversationIdentity, GenerateFunNameError, convo_id_belongs_to_slug, generate_fun_name,
};
