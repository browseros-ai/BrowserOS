use std::{future::Future, time::Duration};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::{
    SolveCtx, config::ForcedMethod, page::CaptchaPage, providers::CaptchaBackend, types::*,
};

const WAIT: Duration = Duration::from_secs(6);
const POLL: Duration = Duration::from_millis(250);

const ANCHOR: &str = r#"/*recaptcha:anchor*/(() => {
 const a=document.querySelector('#recaptcha-anchor'), e=document.querySelector('.recaptcha-checkbox-border')||a;
 const r=e?.getBoundingClientRect();
 return {checked:a?.getAttribute('aria-checked')==='true',error:!!document.querySelector('.recaptcha-checkbox-expired,.recaptcha-checkbox-error'),rect:r?{x:r.x,y:r.y,width:r.width,height:r.height}:null};
})()"#;
const GRID: &str = r#"/*recaptcha:grid*/(() => {
 const t=document.querySelector('table[class*="rc-imageselect-table-"]');if(!t)return null;
 const rect=e=>{const r=e.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height}};
 const v=document.querySelector('#recaptcha-verify-button');if(!v)return null;
 const tiles=[...t.querySelectorAll('td')].map(e=>({rect:rect(e),src:e.querySelector('img')?.src||''}));
 const imgs=[...t.querySelectorAll('img')];
 let ready=!!imgs.length&&imgs.every(i=>i.complete&&i.naturalWidth>0);
 for(let e=t;e;e=e.parentElement){const s=getComputedStyle(e);ready&&=s.display!=='none'&&s.visibility==='visible'&&Number(s.opacity)>.999&&!e.getAnimations().some(a=>a.playState==='running')}
 const d=document.querySelector('.rc-imageselect-desc-wrapper');
 return {label:d?.querySelector('strong')?.innerText?.trim()||'',className:t.className,dynamic:/once there are none left/i.test(d?.innerText||''),ready,
 retry:[...document.querySelectorAll('.rc-imageselect-error-select-more,.rc-imageselect-incorrect-response,.rc-imageselect-error-dynamic-more')].some(e=>e.getBoundingClientRect().height>0),rect:rect(t),verify:rect(v),tiles};
})()"#;
const LOCATE: &str = r#"
 const fs=[...document.querySelectorAll('iframe')], anchors=fs.filter(f=>/\/recaptcha\/.*\/anchor(?:\?|$)/.test(f.src));
 const matches=anchors.filter(f=>f.src===p.anchor),a=matches.length===1?matches[0]:null;
 const visible=e=>{if(!e)return false;const r=e.getBoundingClientRect();if(!r.width||!r.height||r.right<=0||r.bottom<=0||r.left>=innerWidth||r.top>=innerHeight)return false;for(;e;e=e.parentElement){const s=getComputedStyle(e);if(s.display==='none'||s.visibility!=='visible'||Number(s.opacity)<.999||e.getAnimations().some(a=>a.playState==='running'))return false}return true};
 const hidden=e=>{if(!e)return true;const r=e.getBoundingClientRect();if(!r.width||!r.height)return true;for(;e;e=e.parentElement){const s=getComputedStyle(e);if(s.display==='none'||s.visibility==='hidden'||s.visibility==='collapse'||Number(s.opacity)===0)return true}return false};
 let root=null;
 for(let e=a?.parentElement;e;e=e.parentElement){if(anchors.filter(f=>e.contains(f)).length>1)break;if(e.querySelector('textarea[name="g-recaptcha-response"]')){root=e;break}}
 const fields=root?[...root.querySelectorAll('textarea[name="g-recaptcha-response"]')]:[];
"#;
const MAIN: &str = r#"
 let response=fields.some(e=>!!e.value.trim());
 if(!response&&a&&anchors.length===1){try{const g=window.grecaptcha;response=!!(g?.enterprise||g)?.getResponse()?.trim()}catch{}}
 return {response,present:matches.length>0,hidden:matches.length===1?hidden(a):false,frameReady:visible(fs.find(f=>f.src===p.bframe))};
"#;
const APPLY: &str = r#"
 if(!fields.length)return {written:false,callback:false};
 for(const e of fields){e.value=p.token;e.innerHTML=p.token}
 let fn=null,owner=window;
 if(p.callback&&p.callback!=='<inline>'){const parts=p.callback.split('.');fn=window;for(const k of parts){owner=fn;fn=fn?.[k]}}
 if(p.callback==='<inline>'){
   const found=[],seen=new Set();let budget=1000;
   const walk=(o,depth)=>{if(!o||typeof o!=='object'||o.nodeType||seen.has(o)||depth>8||budget--<=0)return;seen.add(o);
     try{if(o.sitekey===p.key){for(const k of ['callback','promise-callback'])if(typeof o[k]==='function')found.push([o,o[k]])}
       for(const v of Object.values(o))walk(v,depth+1)}catch{}};
   walk(window.___grecaptcha_cfg?.clients,0);if(found.length===1)[owner,fn]=found[0];
 }
 try{if(typeof fn==='function'){fn.call(owner,p.token);return {written:true,callback:true}}}catch{}
 return {written:true,callback:false};
"#;
const TOKEN_PRESENT: &str = "return fields.length > 0 && fields.every(e => e.value === p.token);";

#[derive(Deserialize)]
struct Anchor {
    checked: bool,
    error: bool,
    rect: Option<Rect>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Main {
    response: bool,
    present: bool,
    hidden: bool,
    frame_ready: bool,
}
#[derive(Clone, Deserialize)]
struct Tile {
    rect: Rect,
    src: String,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Grid {
    label: String,
    class_name: String,
    dynamic: bool,
    ready: bool,
    retry: bool,
    rect: Rect,
    verify: Rect,
    tiles: Vec<Tile>,
}
impl Grid {
    fn layout(&self) -> Option<TileGrid> {
        [
            ("rc-imageselect-table-33", TileGrid::ThreeByThree),
            ("rc-imageselect-table-44", TileGrid::FourByFour),
        ]
        .into_iter()
        .find(|(class, grid)| {
            self.class_name.split_whitespace().any(|c| c == *class)
                && self.tiles.len() == grid.tiles()
        })
        .map(|(_, grid)| grid)
    }
    fn same(&self, other: &Self, replaced: &[usize]) -> bool {
        self.label == other.label
            && self.class_name == other.class_name
            && self.dynamic == other.dynamic
            && self.tiles.len() == other.tiles.len()
            && self
                .tiles
                .iter()
                .zip(&other.tiles)
                .enumerate()
                .all(|(i, (a, b))| replaced.contains(&i) || a.src == b.src)
    }
}
struct View {
    anchor: Option<FrameRef>,
    state: Option<Anchor>,
    frame: Option<FrameRef>,
    grid: Option<Grid>,
    main: Main,
}
impl View {
    fn passed(&self) -> bool {
        self.main.response && self.state.as_ref().is_some_and(|a| a.checked)
    }
}
struct Stop(SolveStatus, SolveReason, String);
impl From<PageError> for Stop {
    fn from(error: PageError) -> Self {
        let (status, reason) = match error {
            PageError::Closed => (SolveStatus::Cancelled, SolveReason::TabClosed),
            PageError::Navigated => (SolveStatus::Cancelled, SolveReason::Navigated),
            PageError::Cancelled => (SolveStatus::Cancelled, SolveReason::CallerCancelled),
            _ => (SolveStatus::Failed, SolveReason::BrowserError),
        };
        Self(
            status,
            reason,
            "Browser operation could not complete".into(),
        )
    }
}
fn fail(reason: SolveReason, message: &str) -> Stop {
    Stop(SolveStatus::Failed, reason, message.into())
}
fn provider_error(message: &str) -> Stop {
    Stop(
        SolveStatus::NeedsHuman,
        SolveReason::ProviderError,
        message.into(),
    )
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Stop> {
    serde_json::from_value(value)
        .map_err(|_| fail(SolveReason::BrowserError, "Unexpected widget state"))
}
fn frame_matches(frame: &FrameRef, part: &str) -> bool {
    url::Url::parse(&frame.url)
        .is_ok_and(|u| u.path().contains("/recaptcha/") && u.path().ends_with(&format!("/{part}")))
}
fn select_frame(
    frames: &[FrameRef],
    prior: Option<&FrameRef>,
    key: Option<&str>,
    part: &str,
) -> Option<FrameRef> {
    let candidates: Vec<_> = frames
        .iter()
        .filter(|f| {
            frame_matches(f, part)
                && key.is_none_or(|key| {
                    url::Url::parse(&f.url)
                        .is_ok_and(|u| u.query_pairs().any(|(k, v)| k == "k" && v == key))
                })
        })
        .collect();
    candidates
        .iter()
        .find(|f| prior.is_some_and(|p| p.frame_id == f.frame_id))
        .copied()
        .or_else(|| {
            if candidates.len() == 1 {
                candidates.first().copied()
            } else {
                None
            }
        })
        .cloned()
}
fn usable(r: Rect) -> bool {
    [r.x, r.y, r.width, r.height].iter().all(|n| n.is_finite())
        && r.width > 0.
        && r.height > 0.
        && r.x + r.width > 0.
        && r.y + r.height > 0.
}
fn viewport(frame: &FrameRef, local: Rect) -> Result<Rect, Stop> {
    let r = frame
        .rect
        .filter(|r| usable(*r))
        .ok_or_else(|| fail(SolveReason::BrowserError, "Widget frame is hidden"))?;
    if !usable(local)
        || local.x < 0.
        || local.y < 0.
        || local.x + local.width > r.width + 1.
        || local.y + local.height > r.height + 1.
    {
        return Err(fail(
            SolveReason::BrowserError,
            "Widget geometry is unsupported",
        ));
    }
    Ok(Rect {
        x: r.x + local.x,
        y: r.y + local.y,
        ..local
    })
}
// Google's identifiers are documented in CapSolver's ReCaptchaClassification guide.
fn question_id(label: &str) -> Option<&'static str> {
    match label.trim().to_ascii_lowercase().as_str() {
        "traffic lights" | "traffic light" => Some("/m/015qff"),
        "cars" | "car" => Some("/m/0k4j"),
        "crosswalks" | "crosswalk" => Some("/m/014xcs"),
        "bus" | "buses" => Some("/m/01bjv"),
        "school bus" | "school buses" => Some("/m/02yvhj"),
        "bicycles" | "bicycle" => Some("/m/0199g"),
        "motorcycles" | "motorcycle" => Some("/m/04_sv"),
        "fire hydrants" | "fire hydrant" => Some("/m/01pns0"),
        "bridges" | "bridge" => Some("/m/015kr"),
        "boats" | "boat" => Some("/m/019jd"),
        "taxis" | "taxi" => Some("/m/0pg52"),
        "parking meters" | "parking meter" => Some("/m/015qbp"),
        "chimneys" | "chimney" => Some("/m/01jk_4"),
        "tractors" | "tractor" => Some("/m/013xlm"),
        "palm trees" | "palm tree" => Some("/m/0cdl1"),
        "mountains or hills" | "mountains" | "hills" => Some("/m/09d_r"),
        "stairs" => Some("/m/01lynh"),
        _ => None,
    }
}

struct Flow<'a, 'b, P: CaptchaPage, B: CaptchaBackend> {
    ctx: &'a SolveCtx<'b, P, B>,
    challenge: &'a Challenge,
    out: FlowResult,
}
impl<P: CaptchaPage, B: CaptchaBackend> Flow<'_, '_, P, B> {
    fn accepted(&self) -> (SolveMethod, SolveReason) {
        if self.out.attempts.tile_rounds > 0 {
            (SolveMethod::Tiles, SolveReason::TilesAccepted)
        } else {
            (SolveMethod::Checkbox, SolveReason::CheckboxPassed)
        }
    }
    async fn bounded<T>(
        &self,
        future: impl Future<Output = Result<T, PageError>>,
    ) -> Result<T, Stop> {
        tokio::select! {biased;
            _=self.ctx.cancel.cancelled()=>Err(PageError::Cancelled.into()),
            _=tokio::time::sleep_until(self.ctx.deadline)=>Err(fail(SolveReason::DeadlineExceeded,"CAPTCHA deadline exceeded")),
            r=future=>r.map_err(Into::into),
        }
    }
    async fn current(&self) -> Result<(), Stop> {
        if self.bounded(self.ctx.page.document()).await? != self.ctx.document {
            return Err(PageError::Navigated.into());
        }
        Ok(())
    }
    async fn pause(&self, duration: Duration) -> Result<(), Stop> {
        self.bounded(async {
            tokio::time::sleep(duration).await;
            Ok(())
        })
        .await
    }
    fn script(
        &self,
        tag: &str,
        body: &str,
        anchor: Option<&FrameRef>,
        frame: Option<&FrameRef>,
        token: Option<&str>,
    ) -> String {
        let p = json!({"anchor":anchor.or(self.challenge.frame.as_ref()).map(|f|&f.url),"bframe":frame.map(|f|&f.url),
            "key":self.challenge.sitekey,"callback":self.challenge.callback,"token":token});
        format!("/*recaptcha:{tag}*/(() => {{const p={p};{LOCATE}{body}}})()")
    }
    async fn read(&self) -> Result<View, Stop> {
        self.current().await?;
        let frames = self.bounded(self.ctx.page.frames()).await?;
        let anchor = select_frame(
            &frames,
            self.challenge.frame.as_ref(),
            self.challenge.sitekey.as_deref(),
            "anchor",
        );
        let frame = select_frame(
            &frames,
            self.challenge.challenge_frame.as_ref(),
            self.challenge.sitekey.as_deref(),
            "bframe",
        );
        let main: Main = decode(
            self.bounded(self.ctx.page.eval_main(&self.script(
                "state",
                MAIN,
                anchor.as_ref(),
                frame.as_ref(),
                None,
            )))
            .await?,
        )?;
        let state = if let Some(a) = &anchor {
            Some(decode(
                self.bounded(self.ctx.page.eval_in_frame(a, ANCHOR)).await?,
            )?)
        } else {
            None
        };
        let grid = if let Some(f) = frame
            .as_ref()
            .filter(|f| main.frame_ready && f.rect.is_some_and(usable))
        {
            let value = self.bounded(self.ctx.page.eval_in_frame(f, GRID)).await?;
            if value.is_null() {
                None
            } else {
                Some(decode(value)?)
            }
        } else {
            None
        };
        self.current().await?;
        Ok(View {
            anchor,
            state,
            frame,
            grid,
            main,
        })
    }
    async fn wait(&self, until: impl Fn(&View) -> bool) -> Result<View, Stop> {
        let end = Instant::now() + WAIT;
        loop {
            let v = self.read().await?;
            if until(&v) || Instant::now() >= end {
                return Ok(v);
            }
            self.pause(POLL).await?;
        }
    }
    async fn click(&self, frame: &FrameRef, rect: Rect) -> Result<(), Stop> {
        let r = viewport(frame, rect)?;
        let (x, y) = r.center();
        self.current().await?;
        self.bounded(self.ctx.page.click_at(
            x + rand::random_range(-1.0..1.0) * r.width.min(12.) / 12.,
            y + rand::random_range(-1.0..1.0) * r.height.min(12.) / 12.,
        ))
        .await?;
        self.current().await
    }
    fn mark(&mut self, method: SolveMethod, round: u32) {
        self.out.method = method;
        let (step, attempt) = match method {
            SolveMethod::Checkbox => {
                self.out.attempts.checkbox += 1;
                (SolveStep::Checkbox, self.out.attempts.checkbox)
            }
            SolveMethod::Tiles => {
                self.out.attempts.tile_rounds = round;
                (SolveStep::Tiles { round }, round)
            }
            _ => {
                self.out.attempts.token += 1;
                (SolveStep::Token, self.out.attempts.token)
            }
        };
        self.ctx
            .lease
            .set_progress(Some(CaptchaKind::RecaptchaV2), step);
        let mut data = json!({"type":"recaptcha_v2","method":method,"attempt":attempt});
        if method == SolveMethod::Tiles {
            data["round"] = json!(round)
        }
        self.ctx.events.emit(CaptchaEventName::Solving, data);
    }
    async fn paid(&mut self, task: &SolveTask) -> Result<SolveAnswer, Stop> {
        if !self.ctx.backend.is_configured() {
            return Err(Stop(
                SolveStatus::NeedsHuman,
                SolveReason::NoProvider,
                "No CAPTCHA provider configured".into(),
            ));
        }
        let result = tokio::select! {biased;
            _=self.ctx.cancel.cancelled()=>return Err(PageError::Cancelled.into()),
            _=tokio::time::sleep_until(self.ctx.deadline)=>return Err(fail(SolveReason::DeadlineExceeded,"CAPTCHA deadline exceeded")),
            r=self.ctx.backend.solve(task,self.ctx.deadline,self.ctx.cancel)=>r,
        };
        match result {
            Ok(answer) => {
                self.out.paid_tasks += 1;
                if let Some(cost) = answer.cost_usd {
                    if !cost.is_finite() || cost < 0. {
                        return Err(provider_error("Invalid provider cost"));
                    }
                    self.out.cost_usd += cost;
                }
                Ok(answer.answer)
            }
            Err(e) => {
                self.out.paid_tasks += u32::from(e.charged);
                let (status, reason) = match e.kind {
                    ProviderErrorKind::NotConfigured => {
                        (SolveStatus::NeedsHuman, SolveReason::NoProvider)
                    }
                    ProviderErrorKind::Auth
                    | ProviderErrorKind::NoBalance
                    | ProviderErrorKind::Unsupported => {
                        (SolveStatus::NeedsHuman, SolveReason::ProviderUnavailable)
                    }
                    ProviderErrorKind::SpendLimit => {
                        (SolveStatus::NeedsHuman, SolveReason::SpendLimitReached)
                    }
                    ProviderErrorKind::Cancelled => {
                        (SolveStatus::Cancelled, SolveReason::CallerCancelled)
                    }
                    _ => (SolveStatus::NeedsHuman, SolveReason::ProviderError),
                };
                Err(Stop(status, reason, e.message))
            }
        }
    }
    async fn image(
        &self,
        frame: &FrameRef,
        grid: &Grid,
        index: Option<usize>,
        replaced: &[usize],
    ) -> Result<Option<String>, Stop> {
        let tile = index
            .and_then(|i| grid.tiles.get(i))
            .or_else(|| grid.tiles.first())
            .ok_or_else(|| fail(SolveReason::BrowserError, "Empty image grid"))?;
        let src = serde_json::to_string(&tile.src)
            .map_err(|_| fail(SolveReason::BrowserError, "Invalid image source"))?;
        let script = format!(
            r#"/*recaptcha:image*/(async()=>{{const r=await fetch({src});if(!r.ok)throw Error('image fetch');const b=await r.blob();return await new Promise((resolve,reject)=>{{const f=new FileReader();f.onload=()=>resolve(String(f.result).split(',')[1]);f.onerror=reject;f.readAsDataURL(b)}})}})()"#
        );
        match self
            .bounded(self.ctx.page.eval_in_frame(frame, &script))
            .await
        {
            Ok(Value::String(image)) if !image.is_empty() => return Ok(Some(image)),
            Err(e) if e.1 != SolveReason::BrowserError => return Err(e),
            _ => {}
        }
        self.bounded(self.ctx.page.eval_in_frame(frame,
            "/*recaptcha:paint*/new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))))"
        )).await?;
        let fresh = self.read().await?;
        if fresh.passed() {
            return Ok(None);
        }
        let (Some(current_frame), Some(current_grid)) = (fresh.frame, fresh.grid) else {
            return Err(fail(
                SolveReason::BrowserError,
                "Grid disappeared during image capture",
            ));
        };
        if current_frame.frame_id != frame.frame_id
            || !current_grid.ready
            || !grid.same(&current_grid, replaced)
        {
            return Err(fail(
                SolveReason::BrowserError,
                "Grid changed during image capture",
            ));
        }
        self.bounded(self.ctx.page.screenshot_clip(viewport(
            &current_frame,
            index.map_or(current_grid.rect, |i| current_grid.tiles[i].rect),
        )?))
        .await
        .map(Some)
    }
    async fn tiles(&mut self) -> Result<bool, Stop> {
        let mut singles: Option<(String, Grid, Vec<usize>)> = None;
        'rounds: for round in 1..=3 {
            let v = self.read().await?;
            if v.passed() {
                return Ok(true);
            }
            let (Some(frame), Some(grid)) = (v.frame, v.grid.filter(|g| g.ready)) else {
                return Ok(false);
            };
            let Some(layout) = grid.layout() else {
                return Ok(false);
            };
            let tasks: Vec<_> = singles
                .take()
                .filter(|(id, previous, _)| *id == frame.frame_id && previous.same(&grid, &[]))
                .map_or_else(
                    || vec![None],
                    |(_, _, ids)| ids.into_iter().map(Some).collect(),
                );
            let mut clicked = Vec::new();
            for index in tasks {
                let Some(image) = self.image(&frame, &grid, index, &clicked).await? else {
                    return Ok(true);
                };
                let fresh = self.read().await?;
                if fresh.passed() {
                    return Ok(true);
                }
                if fresh
                    .frame
                    .as_ref()
                    .is_none_or(|f| f.frame_id != frame.frame_id)
                    || !fresh
                        .grid
                        .as_ref()
                        .is_some_and(|g| g.ready && grid.same(g, &clicked))
                {
                    continue 'rounds;
                }
                self.mark(SolveMethod::Tiles, round);
                let task = SolveTask::RecaptchaTiles(TilePuzzle {
                    image_base64: image,
                    question_id: question_id(&grid.label).map(str::to_owned),
                    question_text: grid.label.clone(),
                    grid: if index.is_some() {
                        TileGrid::Single
                    } else {
                        layout
                    },
                });
                let SolveAnswer::Tiles(ids) = self.paid(&task).await? else {
                    return Err(provider_error("Expected tile indexes"));
                };
                let limit = if index.is_some() { 1 } else { layout.tiles() };
                if ids
                    .iter()
                    .enumerate()
                    .any(|(i, n)| *n >= limit || ids[..i].contains(n))
                {
                    return Err(provider_error("Invalid tile indexes"));
                }
                for id in ids {
                    let id = index.unwrap_or(id);
                    let fresh = self
                        .wait(|v| {
                            v.passed()
                                || v.frame
                                    .as_ref()
                                    .is_none_or(|f| f.frame_id != frame.frame_id)
                                || v.grid.as_ref().is_some_and(|g| {
                                    g.ready
                                        || !grid.same(g, if grid.dynamic { &clicked } else { &[] })
                                })
                        })
                        .await?;
                    if fresh.passed() {
                        return Ok(true);
                    }
                    let (Some(f), Some(g)) = (fresh.frame, fresh.grid) else {
                        continue 'rounds;
                    };
                    if f.frame_id != frame.frame_id
                        || !g.ready
                        || !grid.same(&g, if grid.dynamic { &clicked } else { &[] })
                    {
                        continue 'rounds;
                    }
                    self.click(&f, g.tiles[id].rect).await?;
                    clicked.push(id);
                    self.pause(Duration::from_millis(120)).await?;
                }
            }
            if grid.dynamic && layout == TileGrid::ThreeByThree && !clicked.is_empty() {
                let next = self
                    .wait(|v| {
                        v.passed()
                            || v.grid.as_ref().is_some_and(|g| {
                                g.ready
                                    && clicked.iter().all(|i| {
                                        g.tiles.get(*i).is_some_and(|t| t.src != grid.tiles[*i].src)
                                    })
                            })
                    })
                    .await?;
                if next.passed() {
                    return Ok(true);
                }
                if !next.grid.as_ref().is_some_and(|g| {
                    g.ready
                        && clicked
                            .iter()
                            .all(|i| g.tiles.get(*i).is_some_and(|t| t.src != grid.tiles[*i].src))
                }) {
                    return Ok(false);
                }
                if round < 3 {
                    singles = next
                        .frame
                        .as_ref()
                        .filter(|f| f.frame_id == frame.frame_id)
                        .and_then(|_| next.grid.filter(|g| grid.same(g, &clicked)))
                        .map(|g| (frame.frame_id.clone(), g, clicked));
                    continue;
                }
            }
            let fresh = self.read().await?;
            if fresh.passed() {
                return Ok(true);
            }
            let (Some(f), Some(g)) = (fresh.frame, fresh.grid) else {
                return Ok(false);
            };
            if f.frame_id != frame.frame_id
                || !g.ready
                || !grid.same(&g, if grid.dynamic { &clicked } else { &[] })
            {
                continue;
            }
            self.click(&f, g.verify).await?;
            self.ctx
                .lease
                .set_progress(Some(CaptchaKind::RecaptchaV2), SolveStep::Verifying);
            let after = self
                .wait(|v| {
                    v.passed()
                        || v.grid
                            .as_ref()
                            .is_some_and(|g| g.ready && (!grid.same(g, &[]) || g.retry))
                })
                .await?;
            if after.passed() {
                return Ok(true);
            }
        }
        Ok(false)
    }
    async fn token(&mut self) -> Result<(SolveMethod, SolveReason), Stop> {
        let fresh = self.read().await?;
        if fresh.passed() {
            return Ok(self.accepted());
        }
        let Some(key) = self
            .challenge
            .sitekey
            .as_ref()
            .filter(|s| !s.trim().is_empty())
        else {
            return Err(Stop(
                SolveStatus::NeedsHuman,
                SolveReason::TilesExhausted,
                "No token parameters after image attempts".into(),
            ));
        };
        if !fresh.main.present {
            return Err(fail(
                SolveReason::AcceptanceUnconfirmed,
                "Widget disappeared before token request",
            ));
        }
        self.mark(SolveMethod::Token, 0);
        let task = SolveTask::RecaptchaToken(RecaptchaParams {
            website_url: self.ctx.page_url.clone(),
            website_key: key.clone(),
            invisible: self.challenge.invisible,
            enterprise: self.challenge.enterprise,
            enterprise_s: self.challenge.recaptcha_s.clone(),
            page_action: self.challenge.action.clone(),
            user_agent: self.ctx.user_agent.clone(),
        });
        let SolveAnswer::Token(token) = self.paid(&task).await? else {
            return Err(provider_error("Expected a response token"));
        };
        if token.trim().is_empty() {
            return Err(provider_error("Empty response token"));
        }
        let before = self.read().await?;
        if before.passed() {
            return Ok(self.accepted());
        }
        if !before.main.present {
            return Err(fail(
                SolveReason::AcceptanceUnconfirmed,
                "Widget disappeared before token application",
            ));
        }
        #[derive(Deserialize)]
        struct Applied {
            written: bool,
            callback: bool,
        }
        let applied: Applied = decode(
            self.bounded(self.ctx.page.eval_main(&self.script(
                "apply",
                APPLY,
                before.anchor.as_ref(),
                before.frame.as_ref(),
                Some(&token),
            )))
            .await?,
        )?;
        if !applied.written {
            return Err(fail(
                SolveReason::AcceptanceUnconfirmed,
                "No response field for this widget",
            ));
        }
        self.out.callback_ran = applied.callback;
        if applied.callback || self.challenge.callback.is_none() {
            self.ctx
                .lease
                .set_progress(Some(CaptchaKind::RecaptchaV2), SolveStep::Verifying);
            let end = Instant::now() + WAIT;
            loop {
                match self.accepted_transition(&before).await {
                    Ok(true) => {
                        return Ok((SolveMethod::Token, SolveReason::TokenAccepted));
                    }
                    Ok(false)
                    | Err(Stop(_, SolveReason::Navigated | SolveReason::BrowserError, _)) => {}
                    Err(error) => return Err(error),
                }
                if Instant::now() >= end {
                    break;
                }
                self.pause(POLL).await?;
            }
            if self.bounded(self.ctx.page.document()).await? == self.ctx.document {
                let retained: bool = decode(
                    self.bounded(self.ctx.page.eval_main(&self.script(
                        "token-present",
                        TOKEN_PRESENT,
                        before.anchor.as_ref(),
                        before.frame.as_ref(),
                        Some(&token),
                    )))
                    .await?,
                )?;
                self.current().await?;
                if !retained {
                    return Err(fail(
                        SolveReason::AcceptanceUnconfirmed,
                        "The applied token is no longer in the form; take a fresh snapshot and retry.",
                    ));
                }
                return Err(Stop(
                    SolveStatus::TokenApplied,
                    SolveReason::AwaitingSubmit,
                    super::token_applied_message(applied.callback),
                ));
            }
        }
        Err(fail(
            SolveReason::AcceptanceUnconfirmed,
            "Token is in the form; page acceptance was not confirmed",
        ))
    }
    async fn accepted_transition(&self, before: &View) -> Result<bool, Stop> {
        let document = self.bounded(self.ctx.page.document()).await?;
        let accepted = if document != self.ctx.document {
            let clear: bool = decode(self.bounded(self.ctx.page.eval_main(
                "/*recaptcha:clear*/document.readyState==='complete'&&!document.querySelector('iframe[src*=\"recaptcha\"],iframe[src*=\"challenges.cloudflare.com\"],iframe[src*=\"hcaptcha\"],.g-recaptcha,.cf-turnstile,.h-captcha')"
            )).await?)?;
            let frames = self.bounded(self.ctx.page.frames()).await?;
            clear
                && !frames.iter().any(|f| {
                    f.url.contains("recaptcha")
                        || f.url.contains("hcaptcha")
                        || f.url.contains("challenges.cloudflare.com")
                })
        } else {
            let now: Main = decode(
                self.bounded(self.ctx.page.eval_main(&self.script(
                    "state",
                    MAIN,
                    before.anchor.as_ref(),
                    before.frame.as_ref(),
                    None,
                )))
                .await?,
            )?;
            !now.present || (!before.main.hidden && now.hidden)
        };
        Ok(self.bounded(self.ctx.page.document()).await? == document && accepted)
    }
    async fn run(&mut self) -> Result<(SolveMethod, SolveReason), Stop> {
        if self.ctx.forced_method == Some(ForcedMethod::Token) {
            return self.token().await;
        }
        let mut v = self.read().await?;
        if v.passed() {
            return Ok((SolveMethod::Checkbox, SolveReason::CheckboxPassed));
        }
        for _ in 0..2 {
            if v.state.as_ref().is_none_or(|a| a.checked)
                || v.grid.as_ref().is_some_and(|g| g.ready)
            {
                break;
            }
            let (Some(frame), Some(rect)) = (
                v.anchor.as_ref().filter(|f| f.rect.is_some_and(usable)),
                v.state.as_ref().and_then(|a| a.rect),
            ) else {
                break;
            };
            self.mark(SolveMethod::Checkbox, 0);
            self.click(frame, rect).await?;
            v = self
                .wait(|v| {
                    v.passed()
                        || v.grid.as_ref().is_some_and(|g| g.ready)
                        || v.state.as_ref().is_some_and(|a| a.error)
                })
                .await?;
            if v.passed() {
                return Ok((SolveMethod::Checkbox, SolveReason::CheckboxPassed));
            }
        }
        if self.tiles().await? {
            return Ok(self.accepted());
        }
        self.token().await
    }
}

pub async fn solve<P: CaptchaPage, B: CaptchaBackend>(
    ctx: &SolveCtx<'_, P, B>,
    challenge: &Challenge,
) -> FlowResult {
    let mut flow = Flow {
        ctx,
        challenge,
        out: FlowResult {
            status: SolveStatus::Failed,
            method: SolveMethod::None,
            reason: SolveReason::BrowserError,
            attempts: Attempts::default(),
            paid_tasks: 0,
            cost_usd: 0.0,
            callback_ran: false,
            message: String::new(),
        },
    };
    match flow.run().await {
        Ok((method, reason)) => {
            flow.out.status = SolveStatus::Solved;
            flow.out.method = method;
            flow.out.reason = reason;
            flow.out.message = "reCAPTCHA acceptance confirmed".into()
        }
        Err(Stop(status, reason, message)) => {
            flow.out.status = status;
            flow.out.reason = reason;
            flow.out.message = message
        }
    }
    flow.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::captcha::{
        events::{CaptchaEventBus, EventLog},
        reservation::TabReservations,
    };
    use serde_json::{Value, json};
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tokio::time::Instant;
    use tokio_util::sync::CancellationToken;

    struct State {
        tiles: bool,
        ignored_checkbox_clicks: u32,
        checked: bool,
        response: bool,
        label: String,
        layout: String,
        dynamic: bool,
        replace_whole_grid: bool,
        replacement_reads: u32,
        replacements: u32,
        tile_replacements: [u32; 9],
        replacement_load_reads: u32,
        loading_tile_reads: u32,
        rejected: u32,
        revision: u32,
        written: bool,
        token_retained: bool,
        callback: bool,
        transition: bool,
        navigation_after_apply: bool,
        navigate_during_verification: bool,
        loading_polls: u32,
        transient_navigation_errors: u32,
        dim_after_apply: bool,
        applied: bool,
        navigated: bool,
        fail: Option<PageError>,
        fetch_fails: bool,
        clear_on_image: bool,
        hide_on_fetch: bool,
        hidden: bool,
        screenshots: Vec<Rect>,
        clicks: Vec<(f64, f64)>,
        scripts: Vec<String>,
    }
    struct FakePage(Arc<Mutex<State>>);
    impl FakePage {
        fn new(tiles: bool) -> Self {
            Self(Arc::new(Mutex::new(State {
                tiles,
                ignored_checkbox_clicks: 0,
                checked: false,
                response: false,
                label: "traffic lights".into(),
                layout: "rc-imageselect-table-33".into(),
                dynamic: false,
                replace_whole_grid: false,
                replacement_reads: 0,
                replacements: 0,
                tile_replacements: [0; 9],
                replacement_load_reads: 0,
                loading_tile_reads: 0,
                rejected: 0,
                revision: 0,
                written: true,
                token_retained: true,
                callback: true,
                transition: true,
                navigation_after_apply: false,
                navigate_during_verification: false,
                loading_polls: 0,
                transient_navigation_errors: 0,
                dim_after_apply: false,
                applied: false,
                navigated: false,
                fail: None,
                fetch_fails: false,
                clear_on_image: false,
                hide_on_fetch: false,
                hidden: false,
                screenshots: vec![],
                clicks: vec![],
                scripts: vec![],
            })))
        }
        fn edit(&self, f: impl FnOnce(&mut State)) {
            f(&mut self.0.lock().unwrap_or_else(|e| e.into_inner()));
        }
        fn read<T>(&self, f: impl FnOnce(&State) -> T) -> T {
            f(&self.0.lock().unwrap_or_else(|e| e.into_inner()))
        }
    }
    fn frame(anchor: bool) -> FrameRef {
        FrameRef {
            frame_id: if anchor { "anchor" } else { "bframe" }.into(),
            url: format!(
                "https://www.google.com/recaptcha/api2/{}?k=test-site-key",
                if anchor { "anchor" } else { "bframe" }
            ),
            rect: Some(if anchor {
                Rect {
                    x: 40.,
                    y: 60.,
                    width: 304.,
                    height: 78.,
                }
            } else {
                Rect {
                    x: 100.,
                    y: 200.,
                    width: 400.,
                    height: 580.,
                }
            }),
        }
    }
    fn document(navigated: bool) -> DocumentId {
        DocumentId {
            url: "https://example.test/form".into(),
            loader_id: if navigated { "next" } else { "initial" }.into(),
        }
    }
    fn challenge() -> Challenge {
        Challenge {
            kind: CaptchaKind::RecaptchaV2,
            state: ChallengeState::Active,
            frame: Some(frame(true)),
            challenge_frame: Some(frame(false)),
            sitekey: Some("test-site-key".into()),
            invisible: false,
            enterprise: false,
            recaptcha_s: None,
            action: None,
            cdata: None,
            callback: Some("passed".into()),
        }
    }
    impl CaptchaPage for FakePage {
        async fn document(&self) -> Result<DocumentId, PageError> {
            self.read(|s| {
                s.fail
                    .clone()
                    .map_or_else(|| Ok(document(s.navigated)), Err)
            })
        }
        async fn frames(&self) -> Result<Vec<FrameRef>, PageError> {
            Ok(self.read(|s| {
                if s.applied && s.transition {
                    vec![]
                } else {
                    let mut frames = vec![frame(true), frame(false)];
                    if s.hidden {
                        for f in &mut frames {
                            f.rect = Some(Rect {
                                x: 1.,
                                y: -9982.5,
                                width: 0.,
                                height: 0.,
                            });
                        }
                    }
                    frames
                }
            }))
        }
        async fn eval_main(&self, expression: &str) -> Result<Value, PageError> {
            if expression.starts_with("/*recaptcha:apply*/") {
                self.edit(|s| {
                    s.applied = true;
                    if s.navigation_after_apply {
                        s.navigated = true;
                    }
                    s.scripts.push(expression.into());
                });
                return Ok(self.read(|s| json!({"written": s.written, "callback": s.callback})));
            }
            if expression.starts_with("/*recaptcha:token-present*/") {
                return Ok(self.read(|s| json!(s.token_retained)));
            }
            if expression.starts_with("/*recaptcha:clear*/") {
                if self.read(|s| s.transient_navigation_errors > 0) {
                    self.edit(|s| s.transient_navigation_errors -= 1);
                    return Err(PageError::Browser("execution context changed".into()));
                }
                let ready = self.read(|s| s.loading_polls == 0);
                self.edit(|s| s.loading_polls = s.loading_polls.saturating_sub(1));
                return Ok(json!(ready));
            }
            self.edit(|s| {
                if s.applied && s.navigate_during_verification {
                    s.navigated = true;
                }
            });
            Ok(self.read(|s| {
                json!({"response": s.response, "present": !(s.applied && s.transition),
                "visible": !(s.applied && (s.transition || s.dim_after_apply)),
                "hidden": s.applied && s.transition, "frameReady": true})
            }))
        }
        async fn eval_in_frame(&self, f: &FrameRef, expression: &str) -> Result<Value, PageError> {
            if expression.starts_with("/*recaptcha:image*/") {
                self.edit(|s| {
                    if s.clear_on_image {
                        s.checked = true;
                        s.response = true;
                    }
                    if s.hide_on_fetch {
                        s.hidden = true;
                    }
                });
                return if self.read(|s| s.fetch_fails) {
                    Err(PageError::Browser("fetch failed".into()))
                } else {
                    Ok(json!("aW1hZ2U="))
                };
            }
            if f.frame_id == "anchor" {
                return Ok(self.read(|s| {
                    json!({"checked": s.checked, "error": false,
                    "rect": {"x": 10., "y": 20., "width": 28., "height": 28.}})
                }));
            }
            self.edit(|s| {
                if s.replace_whole_grid && s.replacements > 0 {
                    s.replacement_reads += 1;
                    if s.replacement_reads == 2 {
                        s.revision += 1;
                    }
                }
            });
            let ready = self.read(|s| s.loading_tile_reads == 0);
            self.edit(|s| s.loading_tile_reads = s.loading_tile_reads.saturating_sub(1));
            Ok(self.read(|s| {
                if !s.tiles || s.clicks.is_empty() || (s.applied && s.transition) { return Value::Null; }
                let tiles: Vec<Value> = (0..9).map(|i| json!({
                    "rect": {"x": 5. + f64::from(i % 3) * 130., "y": 125. + f64::from(i / 3) * 130., "width": 130., "height": 130.},
                    "src": format!("https://www.google.com/payload/{}/{}", s.revision, if s.dynamic { s.tile_replacements[i as usize] } else { 0 }),
                })).collect();
                json!({"label": s.label, "className": s.layout, "dynamic": s.dynamic, "ready": ready,
                    "retry": s.rejected > 0, "rect": {"x": 5., "y": 125., "width": 390., "height": 390.},
                    "verify": {"x": 280., "y": 520., "width": 100., "height": 40.}, "tiles": tiles})
            }))
        }
        async fn click_at(&self, x: f64, y: f64) -> Result<(), PageError> {
            self.edit(|s| {
                s.clicks.push((x, y));
                if x < 100. && !s.tiles {
                    if s.ignored_checkbox_clicks > 0 {
                        s.ignored_checkbox_clicks -= 1;
                    } else {
                        s.checked = true;
                        s.response = true;
                    }
                } else if y > 700. {
                    if s.rejected > 0 {
                        s.rejected -= 1;
                        s.revision += 1;
                    } else {
                        s.checked = true;
                        s.response = true;
                    }
                } else if s.dynamic && y > 200. {
                    s.replacements += 1;
                    let index = ((y - 325.) / 130.) as usize * 3 + ((x - 105.) / 130.) as usize;
                    s.tile_replacements[index] += 1;
                    s.loading_tile_reads = s.replacement_load_reads;
                }
            });
            Ok(())
        }
        async fn screenshot_clip(&self, rect: Rect) -> Result<String, PageError> {
            self.edit(|s| s.screenshots.push(rect));
            Ok("c2NyZWVu".into())
        }
        async fn query_rect_in_frame(
            &self,
            _: &FrameRef,
            _: &str,
        ) -> Result<Option<Rect>, PageError> {
            Ok(None)
        }
    }
    struct FakeBackend {
        answers: Mutex<VecDeque<Result<ProviderAnswer, ProviderError>>>,
        tasks: Mutex<Vec<SolveTask>>,
        after: Option<Arc<Mutex<State>>>,
    }
    impl FakeBackend {
        fn new(answers: Vec<SolveAnswer>) -> Self {
            Self {
                answers: Mutex::new(
                    answers
                        .into_iter()
                        .map(|answer| {
                            Ok(ProviderAnswer {
                                answer,
                                provider: ProviderName::Capsolver,
                                task_id: None,
                                cost_usd: Some(0.002),
                            })
                        })
                        .collect(),
                ),
                tasks: Mutex::new(vec![]),
                after: None,
            }
        }
        fn tasks(&self) -> Vec<SolveTask> {
            self.tasks.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }
    }
    impl CaptchaBackend for FakeBackend {
        fn is_configured(&self) -> bool {
            true
        }
        async fn solve(
            &self,
            task: &SolveTask,
            _: Instant,
            _: &CancellationToken,
        ) -> Result<ProviderAnswer, ProviderError> {
            self.tasks
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(task.clone());
            if let Some(state) = &self.after {
                let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                s.navigated = true;
            }
            self.answers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pop_front()
                .unwrap_or_else(|| {
                    Err(ProviderError {
                        provider: None,
                        kind: ProviderErrorKind::NotConfigured,
                        message: "no canned answer".into(),
                        charged: false,
                    })
                })
        }
    }
    async fn run(
        page: &FakePage,
        backend: &FakeBackend,
        challenge: &Challenge,
        cancel: &CancellationToken,
        deadline: Instant,
    ) -> FlowResult {
        run_with_method(page, backend, challenge, cancel, deadline, None)
            .await
            .0
    }

    async fn run_with_method(
        page: &FakePage,
        backend: &FakeBackend,
        challenge: &Challenge,
        cancel: &CancellationToken,
        deadline: Instant,
        forced_method: Option<super::super::config::ForcedMethod>,
    ) -> (FlowResult, Vec<CaptchaEvent>) {
        let reservations = Arc::<TabReservations>::default();
        let lease = reservations
            .try_reserve(42, "test")
            .unwrap_or_else(|_| panic!("fresh reservation"));
        let events = EventLog::new(CaptchaEventBus::new(), "test".into(), 42);
        let result = solve(
            &SolveCtx {
                page,
                backend,
                events: &events,
                lease: &lease,
                cancel,
                deadline,
                page_url: document(false).url,
                document: document(false),
                user_agent: Some("test-agent".into()),
                forced_method,
            },
            challenge,
        )
        .await;
        (result, events.events())
    }
    async fn attempt(page: &FakePage, backend: &FakeBackend) -> FlowResult {
        run(
            page,
            backend,
            &challenge(),
            &CancellationToken::new(),
            Instant::now() + Duration::from_secs(120),
        )
        .await
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_applied_tokens_await_submit_without_a_visible_transition()
    -> anyhow::Result<()> {
        for callback in [None, Some("passed"), Some("<inline>")] {
            let page = FakePage::new(false);
            page.edit(|state| {
                state.callback = callback.is_some();
                state.transition = false;
            });
            let mut challenge = challenge();
            challenge.callback = callback.map(str::to_owned);
            let (result, _) = run_with_method(
                &page,
                &FakeBackend::new(vec![SolveAnswer::Token("test-token".into())]),
                &challenge,
                &CancellationToken::new(),
                Instant::now() + Duration::from_secs(120),
                Some(super::super::config::ForcedMethod::Token),
            )
            .await;
            assert_eq!(serde_json::to_value(result.status)?, "token_applied");
            assert_eq!(serde_json::to_value(result.reason)?, "awaiting_submit");
            assert_eq!(result.method, SolveMethod::Token);
            assert_eq!(result.paid_tasks, 1);
            assert_eq!(result.callback_ran, callback.is_some());
            assert!(result.message.contains(if callback.is_some() {
                "callback ran"
            } else {
                "no callback was declared"
            }));
            assert!(result.message.to_lowercase().contains("submit"));
            assert!(result.message.contains("solve_captcha"));
            assert!(!result.message.contains("test-token"));
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_cleared_tokens_remain_unconfirmed() {
        let page = FakePage::new(false);
        page.edit(|state| {
            state.transition = false;
            state.token_retained = false;
        });
        let (result, _) = run_with_method(
            &page,
            &FakeBackend::new(vec![SolveAnswer::Token("test-token".into())]),
            &challenge(),
            &CancellationToken::new(),
            Instant::now() + Duration::from_secs(120),
            Some(super::super::config::ForcedMethod::Token),
        )
        .await;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::Failed, SolveReason::AcceptanceUnconfirmed)
        );
        assert!(!result.message.contains("Submit"));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_tokens_without_response_fields_remain_unconfirmed() {
        let page = FakePage::new(false);
        page.edit(|state| state.written = false);
        let (result, _) = run_with_method(
            &page,
            &FakeBackend::new(vec![SolveAnswer::Token("test-token".into())]),
            &challenge(),
            &CancellationToken::new(),
            Instant::now() + Duration::from_secs(120),
            Some(super::super::config::ForcedMethod::Token),
        )
        .await;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::Failed, SolveReason::AcceptanceUnconfirmed)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_forced_tokens_skip_checkbox_and_image_work() {
        for tiles in [false, true] {
            let page = FakePage::new(tiles);
            let backend = FakeBackend::new(vec![SolveAnswer::Token("test-token".into())]);
            let (result, events) = run_with_method(
                &page,
                &backend,
                &challenge(),
                &CancellationToken::new(),
                Instant::now() + Duration::from_secs(120),
                Some(super::super::config::ForcedMethod::Token),
            )
            .await;
            assert_eq!(
                (result.status, result.method, result.reason),
                (
                    SolveStatus::Solved,
                    SolveMethod::Token,
                    SolveReason::TokenAccepted
                )
            );
            assert_eq!(
                result.attempts,
                Attempts {
                    token: 1,
                    ..Default::default()
                }
            );
            assert_eq!(result.paid_tasks, 1);
            assert!(matches!(
                backend.tasks().as_slice(),
                [SolveTask::RecaptchaToken(_)]
            ));
            assert!(page.read(|state| state.clicks.is_empty()
                && state.screenshots.is_empty()
                && state.applied));
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].data["method"], "token");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_forced_recaptcha_tokens_require_the_declared_callback() {
        for (callback, transition) in [(false, true), (false, false)] {
            let page = FakePage::new(false);
            page.edit(|state| {
                state.callback = callback;
                state.transition = transition;
            });
            let (result, _) = run_with_method(
                &page,
                &FakeBackend::new(vec![SolveAnswer::Token("test-token".into())]),
                &challenge(),
                &CancellationToken::new(),
                Instant::now() + Duration::from_secs(120),
                Some(super::super::config::ForcedMethod::Token),
            )
            .await;
            assert_eq!(
                (result.status, result.method, result.reason),
                (
                    SolveStatus::Failed,
                    SolveMethod::Token,
                    SolveReason::AcceptanceUnconfirmed
                )
            );
            assert!(page.read(|state| state.clicks.is_empty() && state.applied));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_checkbox_pass_requires_no_paid_task() {
        let page = FakePage::new(false);
        let backend = FakeBackend::new(vec![]);
        let result = attempt(&page, &backend).await;
        assert_eq!(
            (result.status, result.method, result.reason),
            (
                SolveStatus::Solved,
                SolveMethod::Checkbox,
                SolveReason::CheckboxPassed
            )
        );
        assert_eq!(result.attempts.checkbox, 1);
        assert!(backend.tasks().is_empty());
        let (x, y) = page.read(|s| s.clicks[0]);
        assert!((x - 64.).abs() < 3. && (y - 94.).abs() < 3.);
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_first_tile_round_passes_with_viewport_coordinates() {
        let page = FakePage::new(true);
        let backend = FakeBackend::new(vec![SolveAnswer::Tiles(vec![0, 4])]);
        let result = attempt(&page, &backend).await;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::Solved, SolveReason::TilesAccepted)
        );
        assert_eq!((result.attempts.tile_rounds, result.paid_tasks), (1, 1));
        assert_eq!(result.cost_usd, 0.002);
        let clicks = page.read(|s| s.clicks.clone());
        for (actual, expected) in
            clicks
                .iter()
                .zip([(64., 94.), (170., 390.), (300., 520.), (430., 740.)])
        {
            assert!(
                (actual.0 - expected.0).abs() < 3. && (actual.1 - expected.1).abs() < 3.,
                "{clicks:?}"
            );
        }
        assert_eq!(clicks.len(), 4);
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_rejected_tiles_can_pass_on_the_second_round() {
        let page = FakePage::new(true);
        page.edit(|s| s.rejected = 1);
        let backend = FakeBackend::new(vec![
            SolveAnswer::Tiles(vec![0]),
            SolveAnswer::Tiles(vec![1]),
        ]);
        let result = attempt(&page, &backend).await;
        assert_eq!(result.status, SolveStatus::Solved);
        assert_eq!(result.attempts.tile_rounds, 2);
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_three_failed_rounds_fall_back_to_a_token() {
        let page = FakePage::new(true);
        page.edit(|s| s.rejected = 3);
        let backend = FakeBackend::new(vec![
            SolveAnswer::Tiles(vec![]),
            SolveAnswer::Tiles(vec![]),
            SolveAnswer::Tiles(vec![]),
            SolveAnswer::Token("test-token".into()),
        ]);
        let result = attempt(&page, &backend).await;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::Solved, SolveReason::TokenAccepted)
        );
        assert_eq!(
            (
                result.attempts.tile_rounds,
                result.attempts.token,
                result.paid_tasks
            ),
            (3, 1, 4)
        );
        assert!((result.cost_usd - 0.008).abs() < 0.00001);
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_tokens_with_an_uninvoked_callback_remain_unconfirmed() {
        for (callback, transition) in [(false, false), (false, true)] {
            let page = FakePage::new(true);
            page.edit(|s| {
                s.layout = "rc-imageselect-table-42".into();
                s.callback = callback;
                s.transition = transition;
            });
            let result = attempt(
                &page,
                &FakeBackend::new(vec![SolveAnswer::Token("test-token".into())]),
            )
            .await;
            assert_eq!(
                (result.status, result.reason),
                (SolveStatus::Failed, SolveReason::AcceptanceUnconfirmed)
            );
            assert!(result.message.contains("form"));
        }
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_token_navigation_can_finish_loading_before_acceptance() {
        for (during_verification, errors) in [(false, 0), (true, 0), (false, 1)] {
            let page = FakePage::new(true);
            page.edit(|s| {
                s.layout = "rc-imageselect-table-42".into();
                s.navigation_after_apply = !during_verification;
                s.navigate_during_verification = during_verification;
                s.loading_polls = 2;
                s.transient_navigation_errors = errors;
            });
            let result = attempt(
                &page,
                &FakeBackend::new(vec![SolveAnswer::Token("test-token".into())]),
            )
            .await;
            assert_eq!(
                (result.status, result.reason),
                (SolveStatus::Solved, SolveReason::TokenAccepted)
            );
        }
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_token_navigation_without_acceptance_times_out_unconfirmed() {
        let page = FakePage::new(true);
        page.edit(|s| {
            s.layout = "rc-imageselect-table-42".into();
            s.navigation_after_apply = true;
            s.loading_polls = 100;
        });
        let result = attempt(
            &page,
            &FakeBackend::new(vec![SolveAnswer::Token("test-token".into())]),
        )
        .await;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::Failed, SolveReason::AcceptanceUnconfirmed)
        );
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_dimming_or_scrolling_the_widget_does_not_prove_acceptance() {
        let page = FakePage::new(true);
        page.edit(|s| {
            s.layout = "rc-imageselect-table-42".into();
            s.transition = false;
            s.dim_after_apply = true;
        });
        let result = attempt(
            &page,
            &FakeBackend::new(vec![SolveAnswer::Token("test-token".into())]),
        )
        .await;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::TokenApplied, SolveReason::AwaitingSubmit)
        );
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_provider_failures_do_not_click_tiles() {
        for (kind, reason) in [
            (ProviderErrorKind::NotConfigured, SolveReason::NoProvider),
            (ProviderErrorKind::Auth, SolveReason::ProviderUnavailable),
            (
                ProviderErrorKind::NoBalance,
                SolveReason::ProviderUnavailable,
            ),
            (
                ProviderErrorKind::Unsupported,
                SolveReason::ProviderUnavailable,
            ),
            (
                ProviderErrorKind::SpendLimit,
                SolveReason::SpendLimitReached,
            ),
            (ProviderErrorKind::Unsolvable, SolveReason::ProviderError),
            (ProviderErrorKind::Timeout, SolveReason::ProviderError),
            (ProviderErrorKind::BadRequest, SolveReason::ProviderError),
            (ProviderErrorKind::RateLimited, SolveReason::ProviderError),
            (ProviderErrorKind::Transport, SolveReason::ProviderError),
            (ProviderErrorKind::Uncertain, SolveReason::ProviderError),
        ] {
            let page = FakePage::new(true);
            let backend = FakeBackend::new(vec![]);
            backend
                .answers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_back(Err(ProviderError {
                    provider: Some(ProviderName::Capsolver),
                    kind,
                    message: "ERROR_KEY_DENIED_ACCESS".into(),
                    charged: false,
                }));
            let result = attempt(&page, &backend).await;
            assert_eq!(
                (result.status, result.reason),
                (SolveStatus::NeedsHuman, reason)
            );
            assert_eq!(page.read(|s| s.clicks.len()), 1);
            assert_eq!(backend.tasks().len(), 1);
            if kind == ProviderErrorKind::Auth {
                assert!(result.message.contains("ERROR_KEY_DENIED_ACCESS"));
            }
        }
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_unknown_labels_are_sent_without_a_question_id() {
        let page = FakePage::new(true);
        page.edit(|s| s.label = "airplanes".into());
        let backend = FakeBackend::new(vec![SolveAnswer::Tiles(vec![])]);
        assert_eq!(attempt(&page, &backend).await.status, SolveStatus::Solved);
        assert!(
            matches!(&backend.tasks()[0],SolveTask::RecaptchaTiles(p) if p.question_id.is_none() && p.question_text == "airplanes")
        );
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_navigation_during_a_paid_task_discards_the_answer() {
        let page = FakePage::new(true);
        let mut backend = FakeBackend::new(vec![SolveAnswer::Tiles(vec![0])]);
        backend.after = Some(page.0.clone());
        let result = attempt(&page, &backend).await;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::Cancelled, SolveReason::Navigated)
        );
        assert_eq!(page.read(|s| s.clicks.len()), 1);
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_cancel_and_deadline_stop_before_any_action() {
        for cancelled in [false, true] {
            let page = FakePage::new(false);
            let backend = FakeBackend::new(vec![]);
            let cancel = CancellationToken::new();
            if cancelled {
                cancel.cancel();
            }
            let result = run(
                &page,
                &backend,
                &challenge(),
                &cancel,
                if cancelled {
                    Instant::now() + Duration::from_secs(5)
                } else {
                    Instant::now()
                },
            )
            .await;
            assert_eq!(
                result.reason,
                if cancelled {
                    SolveReason::CallerCancelled
                } else {
                    SolveReason::DeadlineExceeded
                }
            );
            assert!(page.read(|s| s.clicks.is_empty()));
            assert!(backend.tasks().is_empty());
        }
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_page_errors_map_to_terminal_reasons() {
        for (error, reason) in [
            (PageError::Closed, SolveReason::TabClosed),
            (PageError::Navigated, SolveReason::Navigated),
            (PageError::Cancelled, SolveReason::CallerCancelled),
            (PageError::Timeout, SolveReason::BrowserError),
        ] {
            let page = FakePage::new(false);
            page.edit(|s| s.fail = Some(error));
            assert_eq!(
                attempt(&page, &FakeBackend::new(vec![])).await.reason,
                reason
            );
        }
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_already_checked_widget_is_never_paid_or_clicked() {
        let page = FakePage::new(false);
        page.edit(|s| {
            s.checked = true;
            s.response = true;
        });
        let backend = FakeBackend::new(vec![]);
        assert_eq!(attempt(&page, &backend).await.status, SolveStatus::Solved);
        assert!(backend.tasks().is_empty());
        assert!(page.read(|s| s.clicks.is_empty()));
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_malformed_answers_hand_off_before_tile_clicks() {
        for answer in [
            SolveAnswer::Tiles(vec![9]),
            SolveAnswer::Tiles(vec![0, 0]),
            SolveAnswer::Token("unexpected".into()),
        ] {
            let page = FakePage::new(true);
            let result = attempt(&page, &FakeBackend::new(vec![answer])).await;
            assert_eq!(
                (result.status, result.reason),
                (SolveStatus::NeedsHuman, SolveReason::ProviderError)
            );
            assert_eq!(page.read(|s| s.clicks.len()), 1);
        }
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_dynamic_replacements_keep_the_original_tile_position() {
        let page = FakePage::new(true);
        page.edit(|s| s.dynamic = true);
        let backend = FakeBackend::new(vec![
            SolveAnswer::Tiles(vec![0]),
            SolveAnswer::Tiles(vec![0]),
            SolveAnswer::Tiles(vec![]),
        ]);
        let result = attempt(&page, &backend).await;
        assert_eq!(result.status, SolveStatus::Solved);
        assert_eq!(result.attempts.tile_rounds, 3);
        assert_eq!(
            backend
                .tasks()
                .iter()
                .filter(|t| matches!(t,SolveTask::RecaptchaTiles(p) if p.grid==TileGrid::Single))
                .count(),
            2
        );
        let clicks = page.read(|s| s.clicks.clone());
        assert_eq!(clicks.len(), 4);
        assert!((clicks[1].0 - clicks[2].0).abs() < 3. && (clicks[1].1 - clicks[2].1).abs() < 3.);
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_loading_replacements_preserve_remaining_tile_clicks() {
        for loading_reads in [3, 8] {
            let page = FakePage::new(true);
            page.edit(|s| {
                s.dynamic = true;
                s.replacement_load_reads = loading_reads;
            });
            let backend = FakeBackend::new(vec![
                SolveAnswer::Tiles(vec![0, 1]),
                SolveAnswer::Tiles(vec![]),
                SolveAnswer::Tiles(vec![]),
            ]);
            let result = attempt(&page, &backend).await;
            assert_eq!(
                (result.status, result.reason),
                (SolveStatus::Solved, SolveReason::TilesAccepted)
            );
            assert_eq!(result.attempts.tile_rounds, 2);
            assert_eq!(result.attempts.token, 0);
            let clicks = page.read(|s| s.clicks.clone());
            assert_eq!(clicks.len(), 4);
            assert!((clicks[1].0 - 170.).abs() < 3. && (clicks[1].1 - 390.).abs() < 3.);
            assert!((clicks[2].0 - 300.).abs() < 3. && (clicks[2].1 - 390.).abs() < 3.);
        }
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_new_dynamic_grid_is_classified_as_a_whole_grid() {
        let page = FakePage::new(true);
        page.edit(|s| {
            s.dynamic = true;
            s.replace_whole_grid = true;
        });
        let backend = FakeBackend::new(vec![
            SolveAnswer::Tiles(vec![0]),
            SolveAnswer::Tiles(vec![]),
        ]);
        assert_eq!(attempt(&page, &backend).await.status, SolveStatus::Solved);
        assert!(
            matches!(&backend.tasks()[1],SolveTask::RecaptchaTiles(p) if p.grid==TileGrid::ThreeByThree)
        );
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_failed_image_fetch_uses_a_viewport_screenshot() {
        let page = FakePage::new(true);
        page.edit(|s| s.fetch_fails = true);
        let backend = FakeBackend::new(vec![SolveAnswer::Tiles(vec![])]);
        assert_eq!(attempt(&page, &backend).await.status, SolveStatus::Solved);
        assert_eq!(
            page.read(|s| s.screenshots[0]),
            Rect {
                x: 105.,
                y: 325.,
                width: 390.,
                height: 390.
            }
        );
        assert!(
            matches!(&backend.tasks()[0],SolveTask::RecaptchaTiles(p) if p.image_base64=="c2NyZWVu")
        );
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_acceptance_during_capture_prevents_payment() {
        let page = FakePage::new(true);
        page.edit(|s| s.clear_on_image = true);
        let backend = FakeBackend::new(vec![]);
        let result = attempt(&page, &backend).await;
        assert_eq!(
            (result.status, result.method, result.reason),
            (
                SolveStatus::Solved,
                SolveMethod::Checkbox,
                SolveReason::CheckboxPassed
            )
        );
        assert!(backend.tasks().is_empty());
        assert_eq!(page.read(|s| s.clicks.len()), 1);
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_frame_hidden_during_fetch_is_not_screenshotted() {
        let page = FakePage::new(true);
        page.edit(|s| {
            s.fetch_fails = true;
            s.hide_on_fetch = true;
        });
        let backend = FakeBackend::new(vec![]);
        assert_ne!(attempt(&page, &backend).await.status, SolveStatus::Solved);
        assert!(page.read(|s| s.screenshots.is_empty()));
        assert!(backend.tasks().is_empty());
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_hidden_frames_are_never_clicked_or_captured() {
        let page = FakePage::new(true);
        page.edit(|s| s.hidden = true);
        let result = attempt(&page, &FakeBackend::new(vec![])).await;
        assert_ne!(result.status, SolveStatus::Solved);
        assert!(page.read(|s| s.clicks.is_empty() && s.screenshots.is_empty()));
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_checked_without_response_is_not_accepted() {
        let page = FakePage::new(false);
        page.edit(|s| s.checked = true);
        assert_ne!(
            attempt(&page, &FakeBackend::new(vec![])).await.status,
            SolveStatus::Solved
        );
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_exhausted_tiles_without_a_sitekey_request_human_help() {
        let page = FakePage::new(true);
        page.edit(|s| s.rejected = 3);
        let mut ch = challenge();
        ch.sitekey = None;
        let backend = FakeBackend::new(vec![
            SolveAnswer::Tiles(vec![]),
            SolveAnswer::Tiles(vec![]),
            SolveAnswer::Tiles(vec![]),
        ]);
        let result = run(
            &page,
            &backend,
            &ch,
            &CancellationToken::new(),
            Instant::now() + Duration::from_secs(120),
        )
        .await;
        assert_eq!(
            (result.status, result.reason),
            (SolveStatus::NeedsHuman, SolveReason::TilesExhausted)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_an_unregistered_checkbox_click_is_retried_before_payment() {
        let page = FakePage::new(false);
        page.edit(|state| state.ignored_checkbox_clicks = 1);
        let backend = FakeBackend::new(vec![]);
        let result = attempt(&page, &backend).await;
        assert_eq!(
            (result.status, result.method, result.reason),
            (
                SolveStatus::Solved,
                SolveMethod::Checkbox,
                SolveReason::CheckboxPassed
            )
        );
        assert_eq!(result.attempts.checkbox, 2);
        assert_eq!(result.attempts.token, 0);
        assert_eq!(result.paid_tasks, 0);
        assert!(backend.tasks().is_empty());
        assert_eq!(page.read(|state| state.clicks.len()), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_two_unregistered_clicks_exhaust_the_free_retry_before_a_token() {
        let page = FakePage::new(false);
        page.edit(|state| state.ignored_checkbox_clicks = 2);
        let backend = FakeBackend::new(vec![SolveAnswer::Token("test-token".into())]);
        let result = attempt(&page, &backend).await;
        assert_eq!(
            (result.status, result.method, result.reason),
            (
                SolveStatus::Solved,
                SolveMethod::Token,
                SolveReason::TokenAccepted
            )
        );
        assert_eq!(result.attempts.checkbox, 2);
        assert_eq!(result.attempts.token, 1);
        assert_eq!(page.read(|state| state.clicks.len()), 2);
        assert!(matches!(
            backend.tasks().as_slice(),
            [SolveTask::RecaptchaToken(_)]
        ));
    }
}
