//! Task operations. Browser writes are child operations through the same door and rules.

use std::collections::HashSet;
use std::future::{self, Future};

use browseros_core::TabId;
use neo_trace::{Delivery, Effect, TaskId};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::browser::{PageRef, TaskError, TaskRecord};
use crate::dispatch::{
    ErrorCode, LeafError, Listing, Method, MethodSpec, OpCtx, OpFailure, OperationError,
    PageChange, PageTarget, RegistryBuilder, RegistryError, TaskKind,
};

/// Registers task ownership alongside the browser operations it composes.
pub(crate) fn neo_tasks(builder: &mut RegistryBuilder) -> Result<(), RegistryError> {
    builder.define(CreateTask)?;
    builder.define(FetchTask)?;
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
        // A task counts as its creating run's own: it joins the family of the run's host task,
        // whatever the tab scope, so turning task scope on later still shows the run's task
        // what its runs created. A run with no host task makes a family root.
        let created = match op.run().family() {
            Some(family) => op.tabs().create_task_in(name, family),
            None => op.tabs().create_task(name),
        };
        future::ready(
            created
                .map(|task| {
                    op.run().picked_task(&task, TaskKind::Created);
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
        usable_task(op, &input.id)?;
        let listing = op.race(op.run().view().list_pages()).await?;
        let task = usable_task(op, &input.id)?;
        let pages: Vec<PageRef> = task
            .tabs
            .iter()
            .filter(|tab| listing.pages.iter().any(|page| page.tab_id == **tab))
            .filter(|tab| !op.run().closed_it(**tab))
            .map(|tab| PageRef { tab: *tab })
            .collect();
        for page in &pages {
            op.report_page(page, PageChange::Linked);
        }
        op.run().picked_task(&task, TaskKind::Fetched);
        // Built-in `page` takes the task's tab now, not at its first action, so its synchronous
        // getters (`page.url()`, `page.taskId`) are the tab's as soon as this await returns, as
        // Playwright's are once a script holds a page. The tab is one of the Linked pages
        // above, so its fact rides this reply. The reply offers the tab as `pickedPage`; the
        // prelude declines it while the script holds another Page for that tab, and `page`
        // then binds at its first action to this same tab. A task with no open tab leaves
        // `page` lazy: its first action opens a tab in the task, and `about:blank` is the truth.
        if let Some(tab) = task.page_tab(|tab| pages.iter().any(|page| page.tab == tab)) {
            op.bind_run_page(&PageRef { tab });
        }
        Ok(TaskPages { id: task.id, pages })
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
        let task = usable_task(op, &input.task)?;
        let _turn = op.race_classified(task.turn.lock()).await?;
        usable_task(op, &input.task)?;
        let value = op
            .invoke("context.newPage", json!({}))
            .await
            .map_err(child_failure)?;
        let opened: Opened = Opened::deserialize(&value)
            .map_err(|error| OperationError::neo(ErrorCode::INTERNAL_ERROR, error))?;
        let page = PageRef {
            tab: opened.page.tab,
        };
        // A host may have named the task while the tab opened: then it joins no task, and the
        // run is told the task is gone, as for a missing one.
        if !op
            .tabs()
            .own_in_scope(&page, &input.task, op.run().scope())
            .map_err(OperationError::from)?
        {
            return Err(OperationError::from(TaskError::NotFound(input.task)).into());
        }
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
        let task = usable_task(op, &input.task)?;
        let _turn = op.race_classified(task.turn.lock()).await?;
        // Asked again with the turn held, and before each close: a host may name the task
        // while this close waits, and a run with no task never closes a host's task.
        usable_task(op, &input.task)?;
        let mut closed = Vec::new();
        let mut attempted = HashSet::new();
        let mut failure: Option<OpFailure> = None;
        let mut uncertain = false;
        loop {
            let current = usable_task(op, &input.task)?;
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

/// Task `id`, when this run's scope may use it. One it may not pick, open in or close is answered
/// as a missing one, word for word, so the refusal reveals nothing about another task: a bound run
/// uses only its family's tasks, and a run with no task under task scope never a host's.
fn usable_task<M>(op: &OpCtx<M>, id: &TaskId) -> Result<TaskRecord, OperationError> {
    let task = op.tabs().task(id)?;
    if op.run().scope().may_use(&task) {
        Ok(task)
    } else {
        Err(TaskError::NotFound(id.clone()).into())
    }
}

fn child_failure(failure: OpFailure) -> LeafError {
    match failure.delivery {
        Delivery::Unknown => LeafError::ambiguous(failure.error),
        Delivery::Done | Delivery::NotSent => LeafError::definite(failure.error),
    }
}
