//! The one screen and the one window that every active fit shares, as a pure state machine
//! that answers each change with a [`Plan`]; `surface_steps` carries the plan out.
//!
//! Neo has one Xvnc screen and one Chromium window, so their state belongs to the process, not
//! to a fit. The first claim records the window's home state, once: with a record per fit, a
//! second fit would record the first one's fullscreen window as its home and put it back. A
//! window the first claim finds fullscreen goes home maximized, Neo's resting state. Each
//! fit records its tab's viewport at its first claim, and its release puts it back. Only the
//! last release puts the screen and the window home. Zoom cannot be read over CDP, so a
//! release resets it with Ctrl+0 rather than restoring it.
//!
//! Every plan keeps Chromium's order:
//! - the screen before the window, because a fullscreen window takes the screen's size;
//! - the window before the viewport, because Chromium sizes the tab's view to its window;
//! - Ctrl+0 before mobile emulation, because Ctrl+0 sent after it undoes the mobile
//!   preview's zoom-out (decision 24).

use crate::browser_tabs::WindowState;
use crate::fit_rule::{self, Layout};
use crate::{FitId, PartError, ScreenSize, Viewer, ViewerScale};
use browseros_core::viewport::ViewportRequest;
use browseros_core::{TabId, WindowId};

/// One change to the browser or the screen.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Step {
    /// Bring the tab to the front of the screen.
    Activate(TabId),
    /// Resize Xvnc.
    Screen(ScreenSize),
    /// Ctrl+0 on the tab.
    ResetZoom(TabId),
    /// Put the window in this state.
    Window(WindowId, WindowState),
    /// Set the tab's viewport; `None` is the default mode.
    Viewport(TabId, Option<ViewportRequest>),
}

/// Steps in the order they must run.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Plan(pub(crate) Vec<Step>);

impl Plan {
    /// The tabs whose viewport this plan sets.
    pub(crate) fn refitted(&self) -> Vec<TabId> {
        self.0
            .iter()
            .filter_map(|step| match step {
                Step::Viewport(tab, _) => Some(*tab),
                _ => None,
            })
            .collect()
    }
}

/// What a fit's tab and its window showed just before a claim.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Seen {
    /// The window that shows the tab, and its state.
    pub(crate) window: (WindowId, WindowState),
    /// The tab's viewport request; `None` in the default mode.
    pub(crate) viewport: Option<ViewportRequest>,
}

#[derive(Clone, Debug)]
struct Claim {
    fit: FitId,
    tab: TabId,
    window: WindowId,
    /// The tab's viewport before this fit's first claim: what its release puts back.
    recorded: Option<ViewportRequest>,
}

/// Every claim on the surface and what it rested at before the first one. Cloned before a
/// claim runs, so a claim whose screen resize fails puts the state back as it was.
#[derive(Clone, Debug, Default)]
pub(crate) struct Surface {
    /// The window, and its state before the first claim: what the last release puts back.
    home_window: Option<(WindowId, WindowState)>,
    claims: Vec<Claim>,
    /// The layout the surface shows now; `None` while unclaimed.
    shown: Option<Layout>,
}

impl Surface {
    /// Resolve omission where the active scale lives, under the same lock as the claim.
    /// A new fit defaults to 1 even when another fit currently shows a scaled phone.
    pub(crate) fn resolve_request(&self, fit: FitId, viewer: Viewer) -> Result<Layout, PartError> {
        let inherited = self
            .shown
            .filter(|_| self.claims.iter().any(|claim| claim.fit == fit))
            .map_or(ViewerScale::One, |layout| layout.viewer.scale);
        let viewer = viewer.resolve(inherited)?;
        fit_rule::requested_layout(viewer).ok_or(PartError::UnusableViewer(viewer.size))
    }

    /// Interpret the watcher's physical observation using the latest shared preview scale.
    pub(crate) fn observed_layout(&self, screen: ScreenSize) -> Option<Layout> {
        let shown = self.shown?;
        fit_rule::observed_layout(screen, shown.viewer.scale)
    }

    /// `fit` claims the surface for its tab at `layout`'s screen size; a fit already here
    /// claims it again. Records the window's home when nothing held the surface, and the
    /// tab's viewport when the fit is new. When the layout changes, every other claimed tab
    /// follows it too, so all of them agree with the screen.
    pub(crate) fn claim(&mut self, fit: FitId, tab: TabId, layout: Layout, seen: Seen) -> Plan {
        let (window, state) = seen.window;
        if self.claims.is_empty() {
            // Only a fit makes Neo's window fullscreen, so a window already fullscreen with no
            // fit holding the surface is what a Neo that stopped mid-fit left behind (decision
            // 30). Taken for home, the last release would hand the agent the owner's fullscreen.
            let rest = if state == WindowState::Fullscreen {
                WindowState::Maximized
            } else {
                state
            };
            self.home_window = Some((window, rest));
        }
        match self.claims.iter_mut().find(|claim| claim.fit == fit) {
            Some(claim) => claim.window = window,
            None => self.claims.push(Claim {
                fit,
                tab,
                window,
                recorded: seen.viewport,
            }),
        }
        let reshaped = self.shown.is_none_or(|shown| shown.shape != layout.shape);
        self.shown = Some(layout);
        let tabs = self
            .claims
            .iter()
            .map(|claim| claim.tab)
            .filter(|claimed| *claimed == tab || reshaped)
            .collect::<Vec<_>>();
        let mut steps = vec![Step::Activate(tab), Step::Screen(layout.screen)];
        steps.extend(Self::refit(&tabs, window, layout));
        Plan(steps)
    }

    /// The screen is now `layout`'s size, resized by someone else: the owner's noVNC, or
    /// another client. `None` when nothing holds the surface or the size is the one shown.
    /// An empty plan when only the size changed: a maximized window follows the screen by
    /// itself, and the default viewport follows the window. Otherwise every claimed tab is
    /// refitted.
    pub(crate) fn follow_screen(&mut self, layout: Layout) -> Option<Plan> {
        let shown = self.shown?;
        if shown.screen == layout.screen {
            return None;
        }
        self.shown = Some(layout);
        if shown.shape == layout.shape {
            return Some(Plan::default());
        }
        let window = self.claims.first()?.window;
        let tabs = self
            .claims
            .iter()
            .map(|claim| claim.tab)
            .collect::<Vec<_>>();
        Some(Plan(Self::refit(&tabs, window, layout)))
    }

    /// `fit` ends: its tab gets Ctrl+0 and its recorded viewport back. When it was the last
    /// claim, the screen goes back to `home` and the window to its home state first. Unknown
    /// fits get an empty plan, so a release can be repeated.
    pub(crate) fn release(&mut self, fit: FitId, home: ScreenSize) -> Plan {
        let Some(index) = self.claims.iter().position(|claim| claim.fit == fit) else {
            return Plan::default();
        };
        let claim = self.claims.remove(index);
        if !self.claims.is_empty() {
            return Plan(vec![
                Step::ResetZoom(claim.tab),
                Step::Viewport(claim.tab, claim.recorded),
            ]);
        }
        self.shown = None;
        let mut steps = vec![Step::Screen(home), Step::ResetZoom(claim.tab)];
        if let Some((window, state)) = self.home_window.take() {
            steps.push(Step::Window(window, state));
        }
        steps.push(Step::Viewport(claim.tab, claim.recorded));
        Plan(steps)
    }

    /// Whether any fit holds the surface.
    pub(crate) fn is_claimed(&self) -> bool {
        !self.claims.is_empty()
    }

    /// Ctrl+0 on each tab, the window, then each tab's viewport.
    fn refit(tabs: &[TabId], window: WindowId, layout: Layout) -> Vec<Step> {
        let mut steps = tabs
            .iter()
            .map(|tab| Step::ResetZoom(*tab))
            .collect::<Vec<_>>();
        steps.push(Step::Window(window, layout.shape.window()));
        steps.extend(
            tabs.iter()
                .map(|tab| Step::Viewport(*tab, layout.shape.viewport())),
        );
        steps
    }
}

#[cfg(test)]
mod tests {
    use super::{Plan, Seen, Step, Surface};
    use crate::browser_tabs::WindowState;
    use crate::fit_rule::{Layout, layout_for};
    use crate::{FitId, ScreenSize};
    use browseros_core::viewport::{Size, ViewportMode, ViewportRequest};
    use browseros_core::{TabId, WindowId};

    type TestResult = Result<(), &'static str>;

    const HOME: ScreenSize = ScreenSize {
        width: 1600,
        height: 1000,
    };
    const WINDOW: WindowId = WindowId(7);

    fn layout(width: u16, height: u16) -> Result<Layout, &'static str> {
        layout_for(ScreenSize { width, height }).ok_or("a size with a layout")
    }

    /// What a tab in the default mode, in a maximized window, shows.
    fn seen(window: WindowState) -> Seen {
        Seen {
            window: (WINDOW, window),
            viewport: None,
        }
    }

    /// The agent's own mobile emulation, which the fit must put back.
    fn agent_mobile() -> Result<ViewportRequest, &'static str> {
        ViewportRequest::new(ViewportMode::Mobile, Some(150.0))
            .and_then(|request| {
                request.with_size(Size {
                    width: 400,
                    height: 906,
                })
            })
            .map_err(|_| "a valid request")
    }

    #[test]
    fn a_phone_claim_resizes_the_screen_then_the_window_then_the_viewport() -> TestResult {
        let phone = layout(390, 844)?;
        let mut surface = Surface::default();
        let plan = surface.claim(FitId::new(), TabId(3), phone, seen(WindowState::Maximized));
        assert_eq!(
            plan,
            Plan(vec![
                Step::Activate(TabId(3)),
                Step::Screen(phone.screen),
                Step::ResetZoom(TabId(3)),
                Step::Window(WINDOW, WindowState::Fullscreen),
                Step::Viewport(TabId(3), phone.shape.viewport()),
            ])
        );
        assert!(surface.is_claimed());
        Ok(())
    }

    #[test]
    fn the_last_release_puts_the_screen_the_window_and_the_viewport_back() -> TestResult {
        let mut surface = Surface::default();
        let fit = FitId::new();
        surface.claim(
            fit,
            TabId(3),
            layout(390, 844)?,
            Seen {
                window: (WINDOW, WindowState::Maximized),
                viewport: Some(agent_mobile()?),
            },
        );
        assert_eq!(
            surface.release(fit, HOME),
            Plan(vec![
                Step::Screen(HOME),
                Step::ResetZoom(TabId(3)),
                Step::Window(WINDOW, WindowState::Maximized),
                Step::Viewport(TabId(3), Some(agent_mobile()?)),
            ])
        );
        assert!(!surface.is_claimed());
        assert_eq!(
            surface.release(fit, HOME),
            Plan::default(),
            "a second release has nothing left to do"
        );
        Ok(())
    }

    #[test]
    fn the_window_home_is_recorded_once_and_a_refit_keeps_the_first_viewport() -> TestResult {
        let mut surface = Surface::default();
        let (first, second) = (FitId::new(), FitId::new());
        surface.claim(
            first,
            TabId(3),
            layout(390, 844)?,
            seen(WindowState::Maximized),
        );
        // The second fit reads the window the first one made fullscreen; that is not home.
        surface.claim(
            second,
            TabId(4),
            layout(390, 844)?,
            seen(WindowState::Fullscreen),
        );
        // The first fit claims again for a new size; what its tab shows now is its own
        // emulation, which must not become what its release puts back.
        surface.claim(
            first,
            TabId(3),
            layout(412, 915)?,
            Seen {
                window: (WINDOW, WindowState::Fullscreen),
                viewport: layout(412, 915)?.shape.viewport(),
            },
        );
        assert_eq!(
            surface.release(first, HOME),
            Plan(vec![
                Step::ResetZoom(TabId(3)),
                Step::Viewport(TabId(3), None)
            ]),
            "another fit is active: only this tab goes back"
        );
        assert_eq!(
            surface.release(second, HOME),
            Plan(vec![
                Step::Screen(HOME),
                Step::ResetZoom(TabId(4)),
                Step::Window(WINDOW, WindowState::Maximized),
                Step::Viewport(TabId(4), None),
            ])
        );
        Ok(())
    }

    /// Decision 30: a Neo that stopped mid-fit left the window fullscreen, and a fit claimed the
    /// surface before the startup reset could take it out.
    #[test]
    fn a_window_found_fullscreen_goes_home_maximized() -> TestResult {
        let mut surface = Surface::default();
        let fit = FitId::new();
        surface.claim(
            fit,
            TabId(3),
            layout(390, 844)?,
            seen(WindowState::Fullscreen),
        );
        assert_eq!(
            surface.release(fit, HOME),
            Plan(vec![
                Step::Screen(HOME),
                Step::ResetZoom(TabId(3)),
                Step::Window(WINDOW, WindowState::Maximized),
                Step::Viewport(TabId(3), None),
            ])
        );
        Ok(())
    }

    #[test]
    fn a_claim_that_changes_the_layout_refits_every_claimed_tab() -> TestResult {
        let mut surface = Surface::default();
        surface.claim(
            FitId::new(),
            TabId(3),
            layout(390, 844)?,
            seen(WindowState::Maximized),
        );
        let desktop = layout(1280, 800)?;
        assert_eq!(
            surface.claim(
                FitId::new(),
                TabId(4),
                desktop,
                seen(WindowState::Fullscreen)
            ),
            Plan(vec![
                Step::Activate(TabId(4)),
                Step::Screen(desktop.screen),
                Step::ResetZoom(TabId(3)),
                Step::ResetZoom(TabId(4)),
                Step::Window(WINDOW, WindowState::Maximized),
                Step::Viewport(TabId(3), None),
                Step::Viewport(TabId(4), None),
            ])
        );
        let wider = layout(1440, 900)?;
        assert_eq!(
            surface.claim(FitId::new(), TabId(5), wider, seen(WindowState::Maximized)),
            Plan(vec![
                Step::Activate(TabId(5)),
                Step::Screen(wider.screen),
                Step::ResetZoom(TabId(5)),
                Step::Window(WINDOW, WindowState::Maximized),
                Step::Viewport(TabId(5), None),
            ]),
            "the same layout at another size leaves the other tabs alone"
        );
        Ok(())
    }

    #[test]
    fn the_surface_follows_a_screen_someone_else_resized() -> TestResult {
        let mut surface = Surface::default();
        assert_eq!(
            surface.follow_screen(layout(390, 844)?),
            None,
            "nothing holds the surface"
        );
        surface.claim(
            FitId::new(),
            TabId(3),
            layout(390, 844)?,
            seen(WindowState::Maximized),
        );
        assert_eq!(
            surface.follow_screen(layout(390, 844)?),
            None,
            "the size it shows"
        );
        let turned = layout(412, 915)?;
        assert_eq!(
            surface.follow_screen(turned),
            Some(Plan(vec![
                Step::ResetZoom(TabId(3)),
                Step::Window(WINDOW, WindowState::Fullscreen),
                Step::Viewport(TabId(3), turned.shape.viewport()),
            ]))
        );
        let desktop = layout(1280, 800)?;
        assert_eq!(
            surface.follow_screen(desktop),
            Some(Plan(vec![
                Step::ResetZoom(TabId(3)),
                Step::Window(WINDOW, WindowState::Maximized),
                Step::Viewport(TabId(3), None),
            ]))
        );
        assert_eq!(
            surface.follow_screen(layout(1440, 900)?),
            Some(Plan::default()),
            "a maximized window and the default viewport follow the screen by themselves"
        );
        Ok(())
    }
}
