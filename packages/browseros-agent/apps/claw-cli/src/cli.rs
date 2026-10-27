//! The command grammar.
//!
//! Three mismatches separate an MCP tool surface from a CLI surface, and this
//! module exists to fix all three.
//!
//! One: every page-scoped tool needs `page` and every tool accepts `session`.
//! Both come from the scope file, so no command spells them.
//!
//! Two: `act` is seventeen commands wearing one coat. A `kind` discriminator
//! with twenty sibling parameters is right for a tool list, where every extra
//! tool costs schema tokens in every session. It is wrong here, where
//! `--help` is free. It expands into verbs.
//!
//! Three: refs have three spellings. [`crate::refs`] normalizes them.

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "browseros",
    about = "Drive BrowserOS neo from a shell.",
    long_about = "Drive BrowserOS neo from a shell.\n\nThe current page, the session and the tab group are remembered per directory, \
so commands carry no plumbing. Start with `browseros status`, then `browseros open <url>`.\n\n\
Reach for `run` for anything multi-step: it is one call and the only place saved helpers work.",
    version,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(flatten)]
    pub globals: Globals,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Args, Clone, Default)]
pub struct Globals {
    /// Act on this page instead of the remembered one.
    #[arg(long, global = true, value_name = "ID")]
    pub page: Option<u32>,
    /// Server URL. Otherwise resolved from BROWSEROS_URL, runtime.json, then the default.
    #[arg(long, global = true, value_name = "URL")]
    pub endpoint: Option<String>,
    /// The product driving this CLI, for tab labelling: claude, codex, cursor, and so on.
    /// Also read from BROWSEROS_AGENT, which is how a harness sets it once.
    /// Read by hand rather than through clap's `env` feature, which is not
    /// enabled in the workspace and would unify onto claw-server's build.
    #[arg(long, global = true, value_name = "NAME")]
    pub agent: Option<String>,
    /// Wrap the result in a JSON envelope on stdout.
    #[arg(long, global = true)]
    pub json: bool,
    /// Suppress the CLI's own notices on stderr.
    #[arg(long, short, global = true)]
    pub quiet: bool,
    /// Client read timeout in ms. The default is 180000, which must exceed the
    /// server's longest blocking tool.
    #[arg(long, global = true, value_name = "MS")]
    pub timeout: Option<u64>,
    /// Fail instead of pairing a session automatically.
    #[arg(long, global = true)]
    pub no_auto_session: bool,
}

/// How much of `act`'s automatic readback to keep. The tool already returns a
/// post-settle diff, so the CLI never adds a snapshot after an action.
#[derive(Debug, Args, Clone, Default)]
pub struct DiffFlag {
    /// Post-action readback: none, summary, full, or a character cap.
    #[arg(long, value_name = "MODE")]
    pub diff: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Endpoint, browser link, session, current page, and what this server offers.
    Status,
    /// Pair, inspect, list or release the session for this directory.
    #[command(subcommand)]
    Session(SessionCmd),
    /// Set the remembered page.
    Use { page: u32 },

    /// Open a url in a new background tab and remember it.
    Open {
        url: String,
        /// Do not make this the remembered page.
        #[arg(long)]
        keep_page: bool,
    },
    /// Navigate the current page to a url.
    Goto { url: String },
    /// Go back in history.
    Back,
    /// Go forward in history.
    Forward,
    /// Reload the current page.
    Reload,
    /// Recent browser history.
    History {
        #[arg(long, default_value_t = 100)]
        max: u32,
    },

    /// Tabs: list, open, close, or show the active one.
    #[command(subcommand)]
    Tab(TabCmd),
    /// Tab groups.
    #[command(subcommand)]
    Group(GroupCmd),
    /// Browser windows. There is deliberately no activate: an agent must not steal focus.
    #[command(subcommand)]
    Window(WindowCmd),
    /// Browser profiles, where the native supports it.
    #[command(subcommand)]
    Profile(ProfileCmd),
    /// The profile this task is bound to, where the native supports it.
    #[command(subcommand)]
    Target(TargetCmd),

    /// The page as an accessibility tree, with [ref=eN] handles.
    Snapshot {
        /// Actionables plus headings and ancestor context.
        #[arg(long, short = 'i')]
        interactive: bool,
        #[arg(long, value_name = "N")]
        depth: Option<u32>,
    },
    /// What changed since the last snapshot or diff.
    Diff,
    /// Extract the page as markdown, text, links, or console errors.
    Read {
        #[arg(long, value_name = "CSS")]
        selector: Option<String>,
        #[arg(long, conflicts_with_all = ["links", "console"])]
        text: bool,
        #[arg(long, conflicts_with = "console")]
        links: bool,
        #[arg(long)]
        console: bool,
        /// Markdown reads only: visible viewport content.
        #[arg(long)]
        viewport: bool,
        #[arg(long)]
        include_links: bool,
        #[arg(long)]
        include_images: bool,
    },
    /// Search the page without dumping it.
    Grep {
        pattern: String,
        /// "ax" greps snapshot lines and keeps their refs; "content" greps visible text.
        #[arg(long, default_value = "ax")]
        over: String,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Capture the page. Written to a file, because a terminal cannot render an image.
    Screenshot {
        path: Option<String>,
        #[arg(long)]
        full: bool,
        /// Overlay numbered refs from a fresh snapshot.
        #[arg(long)]
        annotate: bool,
        #[arg(long, default_value = "jpeg")]
        format: String,
        #[arg(long)]
        quality: Option<i64>,
    },
    /// Print the page to a PDF.
    Pdf {
        path: Option<String>,
        #[arg(long)]
        landscape: bool,
        #[arg(long)]
        no_background: bool,
    },

    /// Click an element.
    Click {
        #[arg(value_name = "REF")]
        reference: Option<String>,
        /// Click viewport coordinates instead of a ref.
        #[arg(long, value_name = "X,Y", conflicts_with = "reference")]
        at: Option<String>,
        #[arg(long, default_value = "left")]
        button: String,
        #[arg(long, value_name = "N")]
        count: Option<i64>,
        #[command(flatten)]
        diff: DiffFlag,
    },
    /// Fill one field, or a whole form in one call.
    Fill {
        #[arg(value_name = "REF")]
        reference: Option<String>,
        value: Option<String>,
        /// Fill several fields in one call, which is what the tool asks for.
        #[arg(long, value_name = "REF=VALUE", num_args = 1.., conflicts_with_all = ["reference", "value"])]
        fields: Vec<String>,
        /// Append instead of replacing.
        #[arg(long)]
        append: bool,
        /// Read the value from stdin, so it never appears in argv.
        #[arg(long, conflicts_with_all = ["value", "value_env"])]
        value_stdin: bool,
        /// Read the value from this environment variable.
        #[arg(long, value_name = "VAR", conflicts_with = "value")]
        value_env: Option<String>,
        /// Keep the value out of the audit log and skip its screenshot.
        #[arg(long)]
        sensitive: bool,
        #[command(flatten)]
        diff: DiffFlag,
    },
    /// Type into whatever has focus.
    Type {
        text: Option<String>,
        #[arg(long, conflicts_with = "text")]
        value_stdin: bool,
        #[arg(long, value_name = "VAR", conflicts_with = "text")]
        value_env: Option<String>,
        #[arg(long)]
        sensitive: bool,
        #[command(flatten)]
        diff: DiffFlag,
    },
    /// Press a key or combination, for example Enter or Control+a.
    Press {
        key: String,
        #[command(flatten)]
        diff: DiffFlag,
    },
    /// Hover an element.
    Hover {
        #[arg(value_name = "REF")]
        reference: Option<String>,
        #[arg(long, value_name = "X,Y", conflicts_with = "reference")]
        at: Option<String>,
        #[command(flatten)]
        diff: DiffFlag,
    },
    /// Focus an element.
    Focus {
        #[arg(value_name = "REF")]
        reference: String,
        #[command(flatten)]
        diff: DiffFlag,
    },
    /// Check a checkbox.
    Check {
        #[arg(value_name = "REF")]
        reference: String,
        #[command(flatten)]
        diff: DiffFlag,
    },
    /// Uncheck a checkbox.
    Uncheck {
        #[arg(value_name = "REF")]
        reference: String,
        #[command(flatten)]
        diff: DiffFlag,
    },
    /// Choose an option in a select.
    Select {
        #[arg(value_name = "REF")]
        reference: String,
        value: String,
        #[command(flatten)]
        diff: DiffFlag,
    },
    /// Scroll the page or an element.
    Scroll {
        #[arg(default_value = "down")]
        direction: String,
        amount: Option<f64>,
        #[arg(value_name = "REF")]
        reference: Option<String>,
        #[command(flatten)]
        diff: DiffFlag,
    },
    /// Drag one element onto another.
    Drag {
        #[arg(value_name = "FROM")]
        from: String,
        #[arg(value_name = "TO")]
        to: String,
        #[command(flatten)]
        diff: DiffFlag,
    },
    /// Accept or dismiss a pending JavaScript dialog.
    #[command(subcommand)]
    Dialog(DialogCmd),
    /// Set local file paths on a file input.
    Upload {
        #[arg(value_name = "REF")]
        reference: String,
        #[arg(required = true)]
        files: Vec<String>,
    },
    /// Click an element to trigger a download, and report where it landed.
    Download {
        #[arg(value_name = "REF")]
        reference: String,
    },

    /// Wait on a signal. Waiting on the thing beats waiting on the clock.
    Wait {
        #[arg(long, conflicts_with_all = ["selector", "time"])]
        text: Option<String>,
        #[arg(long, conflicts_with = "time")]
        selector: Option<String>,
        /// A fixed pause in ms. A last resort.
        #[arg(long, value_name = "MS")]
        time: Option<f64>,
        /// Polling budget in ms for text and selector.
        #[arg(long, value_name = "MS")]
        timeout: Option<f64>,
    },

    /// Run JavaScript against the browser SDK, in one call. The primary way to drive the browser.
    Run {
        /// The script. Use `-f` for a file or `-` for stdin.
        code: Option<String>,
        #[arg(long, short = 'f', value_name = "FILE", conflicts_with = "code")]
        file: Option<String>,
        /// Do not predefine `page`.
        #[arg(long)]
        no_page: bool,
        #[arg(long, value_name = "MS")]
        script_timeout: Option<f64>,
    },
    /// Evaluate JavaScript inside the page. Prefer `run` for multi-step work.
    Eval {
        code: Option<String>,
        #[arg(long, short = 'f', value_name = "FILE", conflicts_with = "code")]
        file: Option<String>,
        /// Wait for the page to go quiet even if nothing visibly changed.
        #[arg(long)]
        settle: bool,
        #[arg(long, value_name = "N")]
        max_chars: Option<u64>,
        #[arg(long, value_name = "MS")]
        script_timeout: Option<f64>,
    },
    /// Saved per-host helpers. They only work inside `run`.
    #[command(subcommand)]
    Helpers(HelpersCmd),

    /// Saved skills: repeatable tasks neo can re-run by name.
    #[command(subcommand)]
    Skill(SkillCmd),
    /// Ask a human to take over the page.
    #[command(subcommand)]
    Help(HelpCmd),
    /// Pursue a whole outcome on one page. Offered only where the server has a credential.
    Pursue {
        goal: String,
        /// Anything that must be typed, as name=value.
        #[arg(long, value_name = "NAME=VALUE")]
        value: Vec<String>,
        /// Somewhere it may go, as label=url.
        #[arg(long, value_name = "LABEL=URL")]
        destination: Vec<String>,
    },

    /// The CLI's own guides, for an agent learning the API.
    #[command(subcommand)]
    Skills(SkillsCmd),
}

#[derive(Debug, Subcommand)]
pub enum SessionCmd {
    /// Handshake once, adopt or create a tab group, and name the task.
    Start {
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        category: Option<String>,
        #[arg(long)]
        summary: Option<String>,
    },
    /// The same report as `status`.
    Status,
    /// Every scope file on this machine.
    List,
    /// Release the session. Tabs are left open unless asked otherwise.
    End {
        #[arg(long)]
        close_tabs: bool,
        #[arg(long)]
        all: bool,
    },
    /// Name or rename the current task.
    Name {
        name: String,
        #[arg(long)]
        category: Option<String>,
        #[arg(long)]
        summary: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum TabCmd {
    List,
    /// Open a background tab. New tabs never steal the user's focus.
    New {
        url: Option<String>,
    },
    Close {
        page: Option<u32>,
    },
    Active,
}

#[derive(Debug, Subcommand)]
pub enum GroupCmd {
    List,
    Create {
        #[arg(required = true)]
        pages: Vec<u32>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, value_name = "ID")]
        group: Option<String>,
    },
    Update {
        #[arg(value_name = "ID")]
        group: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        color: Option<String>,
        #[arg(long)]
        collapsed: Option<bool>,
    },
    Ungroup {
        #[arg(required = true)]
        pages: Vec<u32>,
    },
    Close {
        #[arg(value_name = "ID")]
        group: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum WindowCmd {
    List,
    Create,
    Close {
        #[arg(value_name = "ID")]
        window: i64,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProfileCmd {
    List,
    Load {
        name: Option<String>,
        #[arg(long, value_name = "ID")]
        profile_id: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum TargetCmd {
    Get,
    Set {
        name: Option<String>,
        #[arg(long, value_name = "ID")]
        profile_id: Option<String>,
    },
    SetWindow {
        #[arg(value_name = "ID")]
        window: i64,
        #[arg(long)]
        strict: bool,
    },
    ClearWindow,
    Clear,
}

#[derive(Debug, Subcommand)]
pub enum DialogCmd {
    Accept,
    Dismiss,
}

#[derive(Debug, Subcommand)]
pub enum HelpersCmd {
    List,
    Read {
        name: String,
    },
    Save {
        name: String,
        #[arg(long, short = 'f', value_name = "FILE")]
        file: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum SkillCmd {
    Save {
        name: String,
        #[arg(long)]
        description: String,
        #[arg(long)]
        steps: Option<String>,
        #[arg(long)]
        site: Option<String>,
        #[arg(long)]
        learned: Option<String>,
    },
    Ran {
        name: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum HelpCmd {
    Request {
        reason: String,
        #[arg(long)]
        details: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        resume_hint: Option<String>,
    },
    Wait {
        #[arg(value_name = "ID")]
        request: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum SkillsCmd {
    List,
    Get {
        #[arg(default_value = "core")]
        name: String,
        #[arg(long)]
        full: bool,
    },
}
