use crate::framework::{FeatureNote, ToolDef, ToolMetadata, input_schema, output_schema};
use browseros_core::NativeFeature;
use rmcp::model::ToolAnnotations;

pub mod act;
pub mod browser_profiles;
pub mod browser_target;
pub mod credentials;
pub mod diff;
pub mod download;
pub mod evaluate;
pub mod grep;
pub mod history;
pub mod navigate;
mod page_creation;
pub mod pdf;
pub mod read;
pub mod run;
pub mod screenshot;
pub mod snapshot;
pub mod tab_groups;
pub mod tabs;
mod target_output_schema;
pub mod upload;
pub mod wait;
pub mod windows;

#[must_use]
pub fn catalog() -> Vec<ToolDef> {
    vec![
        browser_profiles::definition(),
        browser_target::definition(),
        tabs::definition(),
        tab_groups::definition(),
        history::definition(),
        credentials::definition(),
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
        feature_note: None,
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
        feature_note: None,
        handler,
    }
}

/// Profile-targeting documentation shown only to browsers that implement it.
fn profile_note(text: &'static str) -> Option<FeatureNote> {
    Some(FeatureNote {
        feature: NativeFeature::ProfileTargeting,
        text,
    })
}

fn metadata_for_tool(name: &str) -> ToolMetadata {
    ToolMetadata {
        // Both profile tools are hidden together on older natives: a saved selection cannot
        // block ordinary work there (capture runs it unbound), so get/clear are not needed.
        requires: match name {
            "browser_target" | "browser_profiles" => Some(NativeFeature::ProfileTargeting),
            "credentials" => Some(NativeFeature::Credentials),
            _ => None,
        },
        manages_target: matches!(name, "browser_target" | "browser_profiles"),
        accepts_page_arg: matches!(
            name,
            "credentials"
                | "tabs"
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
