use crate::framework::{ToolDef, ToolMetadata, input_schema, output_schema};
use rmcp::model::ToolAnnotations;

pub mod act;
pub mod diff;
pub mod download;
pub mod evaluate;
pub mod grep;
pub mod navigate;
pub mod pdf;
pub mod read;
pub mod run;
pub mod screenshot;
pub mod snapshot;
pub mod tab_groups;
pub mod tabs;
pub mod upload;
pub mod wait;
pub mod windows;

#[must_use]
pub fn catalog() -> Vec<ToolDef> {
    vec![
        tabs::definition(),
        tab_groups::definition(),
        navigate::definition(),
        snapshot::definition(),
        diff::definition(),
        act::definition(),
        download::definition(),
        upload::definition(),
        read::definition(),
        grep::definition(),
        screenshot::definition(),
        pdf::definition(),
        wait::definition(),
        windows::definition(),
        evaluate::definition(),
        run::definition(),
    ]
}

fn read_only_annotations() -> ToolAnnotations {
    ToolAnnotations::new().read_only(true)
}

fn open_world_annotations() -> ToolAnnotations {
    ToolAnnotations::new().open_world(true)
}

fn def<T>(
    name: &'static str,
    description: &'static str,
    annotations: Option<ToolAnnotations>,
    handler: crate::framework::ToolHandler,
) -> ToolDef
where
    T: schemars::JsonSchema + std::any::Any,
{
    ToolDef {
        name,
        description,
        input_schema: input_schema::<T>(),
        output_schema: None,
        annotations,
        metadata: metadata_for_tool(name),
        handler,
    }
}

fn def_with_output<T, O>(
    name: &'static str,
    description: &'static str,
    annotations: Option<ToolAnnotations>,
    handler: crate::framework::ToolHandler,
) -> ToolDef
where
    T: schemars::JsonSchema + std::any::Any,
    O: schemars::JsonSchema + std::any::Any,
{
    ToolDef {
        name,
        description,
        input_schema: input_schema::<T>(),
        output_schema: Some(output_schema::<O>()),
        annotations,
        metadata: metadata_for_tool(name),
        handler,
    }
}

fn metadata_for_tool(name: &str) -> ToolMetadata {
    ToolMetadata {
        accepts_page_arg: matches!(
            name,
            "tabs"
                | "navigate"
                | "snapshot"
                | "diff"
                | "act"
                | "download"
                | "upload"
                | "read"
                | "grep"
                | "screenshot"
                | "pdf"
                | "wait"
                | "evaluate"
        ),
    }
}

fn default_true() -> bool {
    true
}
