pub mod agent_ref;
pub mod ids;
pub mod ownership;
pub mod registry;
pub mod session;

pub use agent_ref::{AgentRef, ClientInfo};
pub use ids::{AgentId, DispatchId, ProfileId, SessionId};
pub use ownership::{AgentKey, AgentPageOwnership};
pub use registry::SessionRegistry;
pub use session::{Session, TabGroupColor, color_for_slug, hex_for_slug};
