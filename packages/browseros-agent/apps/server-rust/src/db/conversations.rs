//! `conversations` rows: the one persisted display history per conversation, tagged with the
//! target that wrote it (`target_type`, `agent_id`) so hydration can tell whose history it is.
//! A port of `apps/server/src/lib/conversations/conversation-store.ts`.
//!
//! Stored messages are returned verbatim ([`StoredConversation::messages`]): a row may hold
//! keys and part types this build does not model, or legacy imports that are not `UIMessage`s at
//! all, and `GET /conversations/{id}` must hand them back unchanged. Only a save rewrites them.

use super::{
    Database, DbError, DbResult,
    agents::AgentKind,
    decode::Columns,
    entities::conversations::{ActiveModel, Column, Entity},
    js,
};
use sea_orm::{
    ActiveValue::Set, ConnectionTrait, DbErr, EntityTrait, FromQueryResult, QueryOrder,
    QueryResult, QuerySelect, sea_query::OnConflict,
};
use serde_json::Value;
use ui_message_stream::UiMessage;
use uuid::Uuid;

/// `MAX_SNIPPET_CHARS`: `last_user_message` keeps this many UTF-16 units.
const LAST_USER_MESSAGE_UTF16_LIMIT: usize = 200;

/// Which target wrote a history (`conversations.target_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetType {
    BrowserOs,
    Claude,
    Codex,
    Custom,
}

impl TargetType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BrowserOs => "browseros",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Custom => "custom",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "browseros" => Some(Self::BrowserOs),
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

impl From<AgentKind> for TargetType {
    fn from(kind: AgentKind) -> Self {
        match kind {
            AgentKind::Claude => Self::Claude,
            AgentKind::Codex => Self::Codex,
            AgentKind::Custom => Self::Custom,
        }
    }
}

/// Lossless persisted target; supported execution targets remain the closed `TargetType`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredTargetType {
    BrowserOs,
    Claude,
    Codex,
    Custom,
    Unknown(String),
}
impl StoredTargetType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::BrowserOs => "browseros",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Custom => "custom",
            Self::Unknown(value) => value,
        }
    }
    fn parse(value: String) -> Self {
        match value.as_str() {
            "browseros" => Self::BrowserOs,
            "claude" => Self::Claude,
            "codex" => Self::Codex,
            "custom" => Self::Custom,
            _ => Self::Unknown(value),
        }
    }
}

/// Raw metadata used by management/diagnostic reads; never normalizes a stored identifier.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredConversationSummary {
    pub id: String,
    pub last_user_message: Option<String>,
    pub origin: Option<String>,
    pub target_type: StoredTargetType,
    pub agent_id: Option<String>,
    pub last_messaged_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Lossless row and raw JSON history, including rows unsupported by chat's execution model.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredConversationRecord {
    pub summary: StoredConversationSummary,
    pub messages: Vec<Value>,
}

/// Row metadata without the message blob. Optional fields are `None` for NULL (TS omits them).
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationSummary {
    pub id: Uuid,
    pub last_user_message: Option<String>,
    pub origin: Option<String>,
    pub target_type: TargetType,
    pub agent_id: Option<String>,
    pub last_messaged_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredConversation {
    pub summary: ConversationSummary,
    /// The stored `UIMessage[]` exactly as written, for `GET /conversations/{id}`.
    pub messages: Vec<Value>,
}

impl StoredConversation {
    /// The typed history; fails if any stored message is not a `UIMessage` (legacy imports are
    /// unvalidated).
    pub fn ui_messages(&self) -> Result<Vec<UiMessage>, serde_json::Error> {
        self.messages
            .iter()
            .map(|message| serde_json::from_value(message.clone()))
            .collect()
    }
}

/// A completed turn's history (insert or replace).
#[derive(Debug, Clone, PartialEq)]
pub struct SaveConversation {
    pub id: Uuid,
    pub messages: Vec<UiMessage>,
    pub origin: Option<String>,
    pub target_type: TargetType,
    pub agent_id: Option<String>,
}

/// [`SaveConversation`] with the messages in their stored JSON form. Chat writes this one: it
/// appends each turn to the row's messages exactly as read, so shapes this build cannot decode
/// (id-less imports, AI SDK v4 parts, `data` messages) survive every save.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveRawConversation {
    pub id: Uuid,
    pub messages: Vec<Value>,
    pub origin: Option<String>,
    pub target_type: TargetType,
    pub agent_id: Option<String>,
}

/// `PUT /conversations/{id}` (legacy import; messages are not validated, arrays included).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportConversation {
    pub messages: Vec<Value>,
    /// Preserved original timestamp; `None` stamps now.
    pub last_messaged_at: Option<i64>,
    /// The route defaults an absent `targetType` to `browseros`.
    pub target_type: TargetType,
    pub origin: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConversationStore {
    db: Database,
}

impl ConversationStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Lossless storage list: ids stay byte-for-byte strings, and future targets stay visible.
    pub async fn list_stored(&self) -> DbResult<Vec<StoredConversationSummary>> {
        // Keep message blobs out of the list just as the typed management list does.
        Ok(Entity::find()
            .select_only()
            .columns([
                Column::Id,
                Column::LastUserMessage,
                Column::Origin,
                Column::TargetType,
                Column::AgentId,
                Column::LastMessagedAt,
                Column::CreatedAt,
                Column::UpdatedAt,
            ])
            .order_by_desc(Column::LastMessagedAt)
            .into_model::<SummaryRecord>()
            .all(self.db.connection())
            .await?
            .into_iter()
            .map(SummaryRecord::stored)
            .collect())
    }

    pub async fn get_stored(&self, id: &str) -> DbResult<Option<StoredConversationRecord>> {
        let Some(row) = Entity::find_by_id(id.to_owned())
            .into_model::<ConversationRecord>()
            .one(self.db.connection())
            .await?
        else {
            return Ok(None);
        };
        let messages = serde_json::from_str(&row.messages).map_err(|error| {
            DbError::Query(format!(
                "conversation {id} has unreadable messages: {error}"
            ))
        })?;
        Ok(Some(StoredConversationRecord {
            summary: row.summary.stored(),
            messages,
        }))
    }

    /// Newest `last_messaged_at` first.
    pub async fn list(&self) -> DbResult<Vec<ConversationSummary>> {
        let rows = Entity::find()
            .select_only()
            .columns([
                Column::Id,
                Column::LastUserMessage,
                Column::Origin,
                Column::TargetType,
                Column::AgentId,
                Column::LastMessagedAt,
                Column::CreatedAt,
                Column::UpdatedAt,
            ])
            .order_by_desc(Column::LastMessagedAt)
            .into_model::<SummaryRecord>()
            .all(self.db.connection())
            .await?;
        rows.into_iter().map(SummaryRecord::summary).collect()
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<StoredConversation>> {
        let Some(row) = Entity::find_by_id(id.to_string())
            .into_model::<ConversationRecord>()
            .one(self.db.connection())
            .await?
        else {
            return Ok(None);
        };
        let messages = serde_json::from_str(&row.messages).map_err(|error| {
            DbError::Query(format!(
                "conversation {id} has unreadable messages: {error}"
            ))
        })?;
        let summary = row.summary.summary()?;
        Ok(Some(StoredConversation { summary, messages }))
    }

    /// Insert or replace; `created_at` survives a replace.
    pub async fn save(&self, input: SaveConversation) -> DbResult<()> {
        let messages = serde_json::to_value(&input.messages)
            .map_err(|error| DbError::Query(format!("conversation messages: {error}")))?;
        let messages = match messages {
            Value::Array(messages) => messages,
            _ => Vec::new(),
        };
        self.save_raw(SaveRawConversation {
            id: input.id,
            messages,
            origin: input.origin,
            target_type: input.target_type,
            agent_id: input.agent_id,
        })
        .await
    }

    /// [`Self::save`] with the messages written verbatim.
    pub async fn save_raw(&self, input: SaveRawConversation) -> DbResult<()> {
        let transaction = self.db.begin_write().await?;
        upsert(
            &transaction,
            Upsert {
                id: input.id,
                messages: &input.messages,
                last_messaged_at: None,
                target_type: input.target_type,
                origin: input.origin,
                agent_id: input.agent_id,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Inserts only when absent, so an import never clobbers a newer server row. `None` when a
    /// row already exists.
    pub async fn import_if_absent(
        &self,
        id: Uuid,
        input: ImportConversation,
    ) -> DbResult<Option<ConversationSummary>> {
        let transaction = self.db.begin_write().await?;
        if exists(&transaction, id).await? {
            return Ok(None);
        }
        let summary = upsert(
            &transaction,
            Upsert {
                id,
                messages: &input.messages,
                last_messaged_at: input.last_messaged_at,
                target_type: input.target_type,
                origin: input.origin,
                agent_id: input.agent_id,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(Some(summary))
    }

    pub async fn delete(&self, id: impl ToString) -> DbResult<bool> {
        let id = id.to_string();
        let transaction = self.db.begin_write().await?;
        let deleted = Entity::delete_by_id(id.to_string())
            .exec(&transaction)
            .await?;
        let deleted = deleted.rows_affected > 0;
        if deleted {
            tracing::info!(conversation_id = %id, "conversation deleted");
        }
        transaction.commit().await?;
        Ok(deleted)
    }
}

struct Upsert<'a> {
    id: Uuid,
    messages: &'a [Value],
    last_messaged_at: Option<i64>,
    target_type: TargetType,
    origin: Option<String>,
    agent_id: Option<String>,
}

async fn exists(connection: &impl ConnectionTrait, id: Uuid) -> Result<bool, DbErr> {
    Ok(Entity::find_by_id(id.to_string())
        .select_only()
        .column(Column::Id)
        .into_tuple::<String>()
        .one(connection)
        .await?
        .is_some())
}

async fn upsert(
    connection: &impl ConnectionTrait,
    input: Upsert<'_>,
) -> DbResult<ConversationSummary> {
    let now = js::now_ms();
    let id = input.id.to_string();
    let created_at = Entity::find_by_id(id.clone())
        .select_only()
        .column(Column::CreatedAt)
        .into_model::<CreatedAt>()
        .one(connection)
        .await?
        .map_or(now, |existing| existing.0);
    let messages = serde_json::to_string(input.messages)
        .map_err(|error| DbError::Query(format!("conversation messages: {error}")))?;
    let summary = ConversationSummary {
        id: input.id,
        last_user_message: last_user_text(input.messages),
        origin: input.origin,
        target_type: input.target_type,
        agent_id: input.agent_id,
        last_messaged_at: input.last_messaged_at.unwrap_or(now),
        created_at,
        updated_at: now,
    };
    let row = ActiveModel {
        id: Set(id),
        messages: Set(messages),
        last_user_message: Set(summary.last_user_message.clone()),
        origin: Set(summary.origin.clone()),
        target_type: Set(summary.target_type.as_str().to_owned()),
        agent_id: Set(summary.agent_id.clone()),
        last_messaged_at: Set(summary.last_messaged_at),
        created_at: Set(created_at),
        updated_at: Set(now),
    };
    Entity::insert(row)
        .on_conflict(
            OnConflict::column(Column::Id)
                .update_columns([
                    Column::Messages,
                    Column::LastUserMessage,
                    Column::Origin,
                    Column::TargetType,
                    Column::AgentId,
                    Column::LastMessagedAt,
                    Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec_without_returning(connection)
        .await?;
    Ok(summary)
}

/// `extractLastUserText`: the newest `role: "user"` message whose text parts, joined by `' '`
/// (other parts contribute `''`) and trimmed, are non-empty; cut to 200 UTF-16 units. Works on
/// raw JSON because imports are not validated; a message without a `parts` array has no text
/// (TS would throw a TypeError there and answer 500).
fn last_user_text(messages: &[Value]) -> Option<String> {
    messages.iter().rev().find_map(|message| {
        if message.get("role").and_then(Value::as_str) != Some("user") {
            return None;
        }
        let joined = message
            .get("parts")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .map(|part| {
                        if part.get("type").and_then(Value::as_str) == Some("text") {
                            js_join_text(part.get("text"))
                        } else {
                            String::new()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let text = js::trim(&joined);
        (!text.is_empty()).then(|| js::utf16_prefix(text, LAST_USER_MESSAGE_UTF16_LIMIT))
    })
}

/// How `Array.prototype.join` renders an element: `null`/`undefined` as `''`.
fn js_join_text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(_) => String::new(),
    }
}

/// Summary columns as stored; `target_type` is checked separately so one unknown value does not
/// fail a whole list.
struct SummaryRecord {
    id: String,
    last_user_message: Option<String>,
    origin: Option<String>,
    target_type: String,
    agent_id: Option<String>,
    last_messaged_at: i64,
    created_at: i64,
    updated_at: i64,
}

impl SummaryRecord {
    fn stored(self) -> StoredConversationSummary {
        StoredConversationSummary {
            id: self.id,
            last_user_message: self.last_user_message,
            origin: self.origin,
            target_type: StoredTargetType::parse(self.target_type),
            agent_id: self.agent_id,
            last_messaged_at: self.last_messaged_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }

    fn summary(self) -> DbResult<ConversationSummary> {
        let id = Uuid::parse_str(&self.id).map_err(|_| {
            DbError::Query(format!(
                "conversation {} has a non-UUID id; use get_stored",
                self.id
            ))
        })?;
        let target_type = TargetType::parse(&self.target_type).ok_or_else(|| {
            DbError::Query(format!(
                "conversation {} has an unsupported target; use get_stored",
                self.id
            ))
        })?;
        Ok(ConversationSummary {
            id,
            last_user_message: self.last_user_message,
            origin: self.origin,
            target_type,
            agent_id: self.agent_id,
            last_messaged_at: self.last_messaged_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

impl FromQueryResult for SummaryRecord {
    fn from_query_result(row: &QueryResult, prefix: &str) -> Result<Self, DbErr> {
        let columns = Columns::new(row, prefix);
        Ok(Self {
            id: columns.text("id")?,
            last_user_message: columns.opt_text("last_user_message")?,
            origin: columns.opt_text("origin")?,
            target_type: columns.text("target_type")?,
            agent_id: columns.opt_text("agent_id")?,
            last_messaged_at: columns.int("last_messaged_at")?,
            created_at: columns.int("created_at")?,
            updated_at: columns.int("updated_at")?,
        })
    }
}

struct CreatedAt(i64);

impl FromQueryResult for CreatedAt {
    fn from_query_result(row: &QueryResult, prefix: &str) -> Result<Self, DbErr> {
        Columns::new(row, prefix).int("created_at").map(Self)
    }
}

struct ConversationRecord {
    summary: SummaryRecord,
    messages: String,
}

impl FromQueryResult for ConversationRecord {
    fn from_query_result(row: &QueryResult, prefix: &str) -> Result<Self, DbErr> {
        Ok(Self {
            summary: SummaryRecord::from_query_result(row, prefix)?,
            messages: Columns::new(row, prefix).text("messages")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::last_user_text;
    use serde_json::json;

    #[test]
    fn last_user_text_follows_extract_last_user_text() {
        let messages = vec![
            json!({"role": "user", "parts": [{"type": "text", "text": "older"}]}),
            json!({"role": "user", "parts": [
                {"type": "text", "text": "  hello"},
                {"type": "file", "url": "x"},
                {"type": "text", "text": "world  "}
            ]}),
            json!({"role": "assistant", "parts": [{"type": "text", "text": "reply"}]}),
            json!({"role": "user", "parts": [{"type": "text", "text": "   "}]}),
            json!({"role": "user"}),
        ];
        assert_eq!(last_user_text(&messages).as_deref(), Some("hello  world"));
        assert_eq!(last_user_text(&[]), None);
        let long = json!({"role": "user", "parts": [{"type": "text", "text": "é".repeat(250)}]});
        assert_eq!(
            last_user_text(&[long]).map(|text| text.chars().count()),
            Some(200)
        );
    }
}
