//! Task operations. Browser writes are child operations through the same door and rules.

use std::collections::HashSet;
use std::future::{self, Future};

use browseros_core::TabId;
use neo_trace::{Delivery, Effect, TaskId};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::browser::PageRef;
use crate::dispatch::{
    ErrorCode, LeafError, Listing, Method, MethodSpec, OpCtx, OpFailure, OperationError,
    PageChange, PageTarget, RegistryBuilder, RegistryError, TaskKind,
};

/// Registers task ownership alongside the browser operations it composes.
pub(crate) fn neo_tasks(builder: &mut RegistryBuilder) -> Result<(), RegistryError> {
    builder.define(CreateTask)?;
    builder.define(FetchTask)?;
    builder.define(Tabs)?;
    builder.define(NewPage)?;
    builder.define(CloseTask)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    #[serde(default, deserialize_with = "supplied_name")]
    name: Option<String>,
}

fn supplied_name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FetchInput {
    id: TaskId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskInput {
    task: TaskId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Serialize)]
struct TaskPages {
    id: TaskId,
    pages: Vec<PageRef>,
}

struct CreateTask;

impl Method for CreateTask {
    type Input = CreateInput;
    type Output = TaskPages;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            listing: Listing::Own(
                "neo.createTask(name?) -> Task: start a task, your own set of tabs; return task.id and keep it",
            ),
            ..MethodSpec::new("neo.createTask", Effect::Read)
        }
    }

    fn target(_input: &CreateInput) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        op: &OpCtx<Self>,
        input: CreateInput,
    ) -> impl Future<Output = Result<TaskPages, LeafError>> + Send {
        let name = input
            .name
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());
        if name.as_ref().is_some_and(|name| name.chars().count() > 200) {
            return future::ready(Err(LeafError::definite(OperationError::script(
                ErrorCode::INVALID_INPUT,
                "a task name must have at most 200 characters after trimming",
            ))));
        }
        future::ready(
            op.tabs()
                .create_task(name)
                .map(|task| {
                    op.run().fetched_task(&task, TaskKind::Created);
                    TaskPages {
                        id: task.id,
                        pages: Vec::new(),
                    }
                })
                .map_err(|error| LeafError::definite(OperationError::from(error))),
        )
    }
}

struct FetchTask;

impl Method for FetchTask {
    type Input = FetchInput;
    type Output = TaskPages;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            listing: Listing::Own(
                "neo.task(id) -> Task: a task from an earlier run (task.id, task.pages(), task.page(tabId), task.newPage(), task.close())",
            ),
            ..MethodSpec::new("neo.task", Effect::Read)
        }
    }

    fn target(_input: &FetchInput) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, op: &OpCtx<Self>, input: FetchInput) -> Result<TaskPages, LeafError> {
        // A listing reconciles tabs closed outside Neo before returning task membership.
        op.tabs().task(&input.id).map_err(OperationError::from)?;
        let listing = op.race(op.run().view().list_pages()).await?;
        let task = op.tabs().task(&input.id).map_err(OperationError::from)?;
        let pages: Vec<PageRef> = task
            .tabs
            .iter()
            .filter(|tab| listing.pages.iter().any(|page| page.tab_id == **tab))
            .map(|tab| PageRef { tab: *tab })
            .collect();
        for page in &pages {
            op.report_page(page, PageChange::Linked);
        }
        op.run().fetched_task(&task, TaskKind::Fetched);
        Ok(TaskPages { id: task.id, pages })
    }
}

struct Tabs;

impl Method for Tabs {
    type Input = Empty;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            listing: Listing::Own(
                "neo.tabs() -> Page[]: every open tab, each with page.tabId and page.taskId (null for the user's own tabs)",
            ),
            ..MethodSpec::new("neo.tabs", Effect::Read)
        }
    }

    fn target(_input: &Empty) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, op: &OpCtx<Self>, _input: Empty) -> Result<Value, LeafError> {
        let listing = op.race(op.run().view().list_pages()).await?;
        let pages: Vec<PageRef> = listing.pages.iter().map(PageRef::from).collect();
        for page in &pages {
            op.report_page(page, PageChange::Linked);
        }
        Ok(json!({ "pages": pages }))
    }
}

#[derive(Deserialize)]
struct Opened {
    page: OpenedTab,
}
#[derive(Deserialize)]
struct OpenedTab {
    tab: TabId,
}

struct NewPage;

impl Method for NewPage {
    type Input = TaskInput;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            listing: Listing::Hidden,
            ..MethodSpec::new("task.newPage", Effect::Write)
        }
    }

    fn target(_input: &TaskInput) -> Option<&PageTarget> {
        None
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "the task turn serializes opens against closes through child operations"
    )]
    async fn call(&self, op: &OpCtx<Self>, input: TaskInput) -> Result<Value, LeafError> {
        let task = op.tabs().task(&input.task).map_err(OperationError::from)?;
        let _turn = op.race_classified(task.turn.lock()).await?;
        op.tabs().task(&input.task).map_err(OperationError::from)?;
        let value = op
            .invoke("context.newPage", json!({}))
            .await
            .map_err(child_failure)?;
        let opened: Opened = Opened::deserialize(&value)
            .map_err(|error| OperationError::neo(ErrorCode::INTERNAL_ERROR, error))?;
        let page = PageRef {
            tab: opened.page.tab,
        };
        op.tabs()
            .own(&page, &input.task)
            .map_err(OperationError::from)?;
        op.report_page(&page, PageChange::Linked);
        Ok(value)
    }
}

struct CloseTask;

impl Method for CloseTask {
    type Input = TaskInput;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            listing: Listing::Hidden,
            ..MethodSpec::new("task.close", Effect::Write)
        }
    }

    fn target(_input: &TaskInput) -> Option<&PageTarget> {
        None
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "the task turn keeps a concurrent open from racing the final close"
    )]
    async fn call(&self, op: &OpCtx<Self>, input: TaskInput) -> Result<Value, LeafError> {
        let task = op.tabs().task(&input.task).map_err(OperationError::from)?;
        let _turn = op.race_classified(task.turn.lock()).await?;
        let mut closed = Vec::new();
        let mut attempted = HashSet::new();
        let mut failure: Option<OpFailure> = None;
        let mut uncertain = false;
        loop {
            let current = op.tabs().task(&input.task).map_err(OperationError::from)?;
            let Some(tab) = current
                .tabs
                .into_iter()
                .find(|tab| !attempted.contains(tab))
            else {
                if let Some(failure) = failure {
                    return Err(if uncertain {
                        LeafError::ambiguous(failure.error)
                    } else {
                        child_failure(failure)
                    });
                }
                if op.tabs().forget_task(&task.id) {
                    op.run().closed_task(&task);
                    return Ok(json!({ "closed": closed }));
                }
                // A popup joined between the snapshot and the atomic empty-task removal.
                continue;
            };
            attempted.insert(tab);
            let page = PageRef { tab };
            match op.invoke("page.close", json!({ "page": page })).await {
                Ok(_) => {
                    closed.push(tab);
                    op.report_page(&page, PageChange::Closed);
                }
                Err(error) => {
                    uncertain |= error.delivery == Delivery::Unknown;
                    failure.get_or_insert(error);
                }
            }
        }
    }
}

fn child_failure(failure: OpFailure) -> LeafError {
    match failure.delivery {
        Delivery::Unknown => LeafError::ambiguous(failure.error),
        Delivery::Done | Delivery::NotSent => LeafError::definite(failure.error),
    }
}
