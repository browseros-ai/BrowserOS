//! Picture decisions driven by observations and caller-supplied time.
use std::collections::VecDeque;
use std::fmt;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Full,
    Half,
}

#[derive(Clone, Copy)]
pub struct Config {
    pub base_crf: f32,
    pub crf_range: f32,
    pub sharpen_qp: u8,
    pub sharpen_after: Duration,
    pub max_fps: f32,
    pub adaptive_size: bool,
    pub cpu_guard: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rung {
    fps: f32,
    crf_add: f32,
    scale: Scale,
}

/// Both pressure sources choose from the same ordered picture settings.
struct Ladder {
    rungs: Vec<Rung>,
    floor: usize,
    cap: usize,
    half: Option<usize>,
    low: Option<usize>,
}

impl Ladder {
    fn new(cfg: Config) -> Self {
        let top = Rung {
            fps: cfg.max_fps,
            crf_add: 0.0,
            scale: Scale::Full,
        };
        let mut rungs = vec![top];
        for fps in [30.0, 20.0, 15.0] {
            if fps < cfg.max_fps {
                rungs.push(Rung { fps, ..top });
            }
        }
        let floor = rungs.len() - 1;
        let full = rungs[floor];
        let mut crf_add = 0.0;
        while crf_add < cfg.crf_range {
            crf_add = (crf_add + 2.0_f32).min(cfg.crf_range);
            rungs.push(Rung { crf_add, ..full });
        }
        let cap = rungs.len() - 1;
        let half = cfg.adaptive_size.then(|| {
            rungs.push(Rung {
                scale: Scale::Half,
                ..rungs[cap]
            });
            rungs.len() - 1
        });
        let low = (10.0 <= 0.8 * full.fps).then(|| {
            rungs.push(Rung {
                fps: 10.0,
                ..rungs[half.unwrap_or(floor)]
            });
            rungs.len() - 1
        });
        Self {
            rungs,
            floor,
            cap,
            half,
            low,
        }
    }
}

/// A pressure source owns its position and the evidence for its next move.
struct Level {
    at: usize,
    since: Instant,
    pressure: u8,
    clean: u8,
    need: u8,
    last_up: Option<Instant>,
}

impl Level {
    fn new(now: Instant) -> Self {
        Self {
            at: 0,
            since: now,
            pressure: 0,
            clean: 0,
            need: 3,
            last_up: None,
        }
    }

    fn move_to(&mut self, at: usize, now: Instant) {
        self.at = at;
        self.since = now;
        self.pressure = 0;
        self.clean = 0;
    }

    fn under_pressure(&mut self, now: Instant) {
        self.clean = 0;
        if recent(self.last_up.take(), now) {
            self.need = (self.need * 2).min(24);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Link,
    Cpu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    Congested,
    Queue,
    Reset,
    Severe,
    Work,
    Steal,
    Throttled,
    Jam,
    Clean,
    Calm,
    Capacity,
    Trial,
}

impl Why {
    fn label(self) -> &'static str {
        match self {
            Self::Congested => "congested",
            Self::Queue => "queue",
            Self::Reset => "reset",
            Self::Severe => "severe",
            Self::Work => "work",
            Self::Steal => "steal",
            Self::Throttled => "throttled",
            Self::Jam => "jam",
            Self::Clean => "clean",
            Self::Calm => "calm",
            Self::Capacity => "capacity",
            Self::Trial => "trial",
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Facts {
    blocked: f32,
    queue_ms: u32,
    offered_kbps: f32,
    delivered_kbps: f32,
    work_ms: f32,
    stolen: f32,
    frames: u32,
    grabs: u32,
    busy: Duration,
}

/// One source changed its vote. The logged settings are the effective picture after that vote.
pub struct Move {
    source: Source,
    from: usize,
    to: usize,
    rung: Rung,
    base_crf: f32,
    why: Why,
    facts: Facts,
}

impl fmt::Display for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let source = match self.source {
            Source::Link => "link",
            Source::Cpu => "cpu",
        };
        let scale = match self.rung.scale {
            Scale::Full => "full",
            Scale::Half => "half",
        };
        write!(
            f,
            "source={source} from={} to={} fps={} crf={} scale={scale} why={} blocked={:.0} queue_ms={} offered_kbps={:.0} delivered_kbps={:.0} work_ms={} stolen={:.0} frames={} busy_ms={} grabs={}",
            self.from,
            self.to,
            self.rung.fps,
            self.base_crf + self.rung.crf_add,
            self.why.label(),
            self.facts.blocked * 100.0,
            self.facts.queue_ms,
            self.facts.offered_kbps,
            self.facts.delivered_kbps,
            self.facts.work_ms,
            self.facts.stolen * 100.0,
            self.facts.frames,
            self.facts.busy.as_millis(),
            self.facts.grabs
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub scale: Scale,
    pub sharp_qp: Option<u8>,
}

#[derive(Clone, Copy, Default)]
pub struct Work {
    pub grab: Duration,
    pub convert: Duration,
    pub encode: Duration,
}

pub struct Encoded {
    pub plan: Plan,
    pub key: bool,
    pub owner_key: bool,
    pub bytes: usize,
    pub pixels: usize,
    pub big: bool,
    pub work: Work,
}

#[derive(Default)]
pub struct Link {
    pub blocked: f32,
    pub sent_kbps: f32,
    pub delivered_kbps: f32,
    pub queue_ms: u32,
    /// Median ACK delay above the path's base RTT. Late reset ACKs can deliver bytes without a delay.
    pub base_queue_ms: Option<u32>,
    /// Frames sent during this observation interval.
    pub frames: u32,
    /// Completed screen grabs, including those that found unchanged pixels.
    pub grabs: u32,
    /// Capture wanted damage at some point during this interval, including while blocked.
    pub damage_pending: bool,
    /// Time with damage pending and credit to send a frame to the pacer.
    pub busy: Duration,
}

#[derive(Default)]
pub struct Cpu {
    pub stolen: f32,
    pub throttled: bool,
}

const FULL_SIZE_AFTER: Duration = Duration::from_secs(1);
const CPU_FULL_SIZE_AFTER: Duration = Duration::from_secs(2);
const CAPACITY_WINDOW: Duration = Duration::from_secs(10);
const STALE_CAPACITY_AFTER_JAM: Duration = Duration::from_secs(20);

fn capacity_expiry(at: Instant, now: Instant) -> Option<Instant> {
    // capacity_kbps still includes samples exactly ten seconds old.
    let expiry = at + CAPACITY_WINDOW + Duration::from_nanos(1);
    (now < expiry).then_some(expiry)
}

// Integer nanoseconds keep the 40 ms recovery boundary at 15 fps inclusive.
fn starved_work(fps: f32) -> Duration {
    Duration::from_nanos((800_000_000.0 / f64::from(fps)) as u64)
}

fn full_work(fps: f32) -> Duration {
    Duration::from_nanos((600_000_000.0 / f64::from(fps)) as u64)
}

fn recent(at: Option<Instant>, now: Instant) -> bool {
    at.is_some_and(|at| now.saturating_duration_since(at) <= Duration::from_secs(10))
}

impl Scale {
    /// H.264 4:2:0 needs even dimensions at either scale. Rounding at half size can discard
    /// two more screen pixels than encoding the same screen at full size.
    pub fn dims(self, width: u16, height: u16) -> (u16, u16) {
        let divisor = match self {
            Self::Full => 1,
            Self::Half => 2,
        };
        ((width / divisor) & !1, (height / divisor) & !1)
    }
}

impl Work {
    fn total(self) -> Duration {
        self.grab + self.convert + self.encode
    }
    fn full(self, scale: Scale) -> Duration {
        match scale {
            Scale::Full => self.total(),
            Scale::Half => self.grab + self.convert.mul_f32(1.3) + self.encode * 4,
        }
    }
}

struct Idr {
    bytes: usize,
    qp: f32,
    pixels: usize,
}

pub struct Picture {
    cfg: Config,
    ladder: Ladder,
    link: Level,
    cpu: Level,
    width: usize,
    soft: Vec<bool>,
    soft_minimum: usize,
    scale: Scale,
    /// The encoded half frame keeps its settle delay even if a level moves before the next send.
    encoded_cpu_half: bool,
    link_kbps: u32,
    last_big: Instant,
    last_change: Instant,
    newest_damage: Option<Instant>,
    planned_change: Option<Instant>,
    calm_sharp: bool,
    last_input: Option<Instant>,
    queue_high: bool,
    previous_late_at_cap: bool,
    previous_congested_no_ack: bool,
    last_reset: Option<Instant>,
    reset_since_tick: bool,
    /// An asynchronous reset reports the last complete interval's measurements.
    facts: Facts,
    last_pressure: Option<Instant>,
    last_congested: Option<(Instant, f32)>,
    /// Zero once near-base ACKs have released the jam's return-rate limits.
    clear_ticks_needed: u8,
    last_full: Option<Instant>,
    backoff: Duration,
    capacity: VecDeque<(Instant, f32)>,
    last_probe: Option<(Instant, f32)>,
    /// Full-size returns keep the last nonzero delivery through a stall.
    last_congested_kbps: Option<f32>,
    last_delivery: Option<(Instant, f64)>,
    demand_kbps: Option<f32>,
    full_big: usize,
    full_big_bytes: u64,
    cpu_trial: bool,
    cpu_calm_needed: u8,
    cpu_return_pending: bool,
    cpu_returned_at: Option<Instant>,
    cpu_full_calm_since: Option<Instant>,
    cpu_full_floor: Option<(Instant, Duration)>,
    last_starved: Instant,
    sampled_full: usize,
    work: Vec<Duration>,
    full_work: Vec<Duration>,
    predicted_full: Option<Duration>,
    pub work_p75_ms: f32,
    idr: Option<Idr>,
}

impl Picture {
    pub fn new(cfg: Config, width: usize, height: usize, now: Instant) -> Self {
        Self {
            cfg,
            ladder: Ladder::new(cfg),
            link: Level::new(now),
            cpu: Level::new(now),
            width,
            soft: vec![false; height],
            soft_minimum: 0,
            scale: Scale::Full,
            encoded_cpu_half: false,
            link_kbps: 2000,
            last_big: now,
            last_change: now,
            newest_damage: None,
            planned_change: None,
            calm_sharp: false,
            last_input: None,
            queue_high: false,
            previous_late_at_cap: false,
            previous_congested_no_ack: false,
            last_reset: None,
            reset_since_tick: false,
            facts: Facts::default(),
            last_pressure: None,
            last_congested: None,
            clear_ticks_needed: 0,
            last_full: None,
            backoff: Duration::from_secs(2),
            capacity: VecDeque::new(),
            last_probe: None,
            last_congested_kbps: None,
            last_delivery: None,
            demand_kbps: None,
            full_big: 0,
            full_big_bytes: 0,
            cpu_trial: false,
            cpu_calm_needed: 3,
            cpu_return_pending: false,
            cpu_returned_at: None,
            cpu_full_calm_since: None,
            cpu_full_floor: None,
            last_starved: now,
            sampled_full: 0,
            work: Vec::new(),
            full_work: Vec::new(),
            predicted_full: None,
            work_p75_ms: 0.0,
            idr: None,
        }
    }

    pub fn resize(&mut self, width: usize, height: usize, now: Instant) {
        self.width = width;
        self.soft = vec![false; height];
        self.soft_minimum = 0;
        self.last_big = now;
        self.last_change = now;
        self.newest_damage = None;
        self.planned_change = None;
        self.last_delivery = None;
        self.calm_sharp = false;
        self.demand_kbps = None;
        self.idr = None;
        self.predicted_full = None;
        self.cpu_full_floor = None;
        self.cpu_full_calm_since = None;
        self.work.clear();
        self.full_work.clear();
        self.sampled_full = 0;
        self.full_big = 0;
        self.full_big_bytes = 0;
    }

    pub fn crf(&self) -> f32 {
        self.cfg.base_crf + self.ladder.rungs[self.link.at].crf_add
    }
    pub fn fps(&self) -> f32 {
        self.ladder.rungs[self.link.at]
            .fps
            .min(self.ladder.rungs[self.cpu.at].fps)
    }
    pub fn rung_label(&self) -> String {
        format!("link {} cpu {}", self.link.at, self.cpu.at)
    }

    /// A new pacer's acknowledgements start a fresh run of evidence at the current settings.
    pub fn pacer_changed(&mut self) {
        self.link.pressure = 0;
        self.link.clean = 0;
        self.link.need = 3;
        self.link.last_up = None;
        self.last_reset = None;
        self.reset_since_tick = false;
        self.queue_high = false;
        self.previous_late_at_cap = false;
        self.previous_congested_no_ack = false;
        self.last_congested = None;
        self.clear_ticks_needed = 0;
        self.capacity.clear();
        self.last_probe = None;
        self.last_congested_kbps = None;
        self.last_delivery = None;
        self.facts = Facts::default();
    }

    pub fn link_kbps(&self) -> u32 {
        self.link_kbps
    }
    pub fn scale(&self) -> Scale {
        self.scale
    }
    pub fn scale_label(&self) -> &'static str {
        match self.scale {
            Scale::Full => "full",
            Scale::Half if self.encoded_cpu_half => "half(cpu)",
            Scale::Half => "half(link)",
        }
    }

    pub fn input(&mut self, now: Instant) {
        self.last_input = Some(now);
    }

    pub fn damage(&mut self, now: Instant) {
        self.newest_damage = Some(now);
    }

    /// Large changes and forced keys use the size chosen by the link and CPU.
    pub fn motion_scale(&self) -> Scale {
        if self.ladder.rungs[self.cpu.at].scale == Scale::Half
            || self.ladder.rungs[self.link.at].scale == Scale::Half
        {
            Scale::Half
        } else {
            Scale::Full
        }
    }

    pub fn plan(&mut self, now: Instant, band: Option<(usize, usize, usize)>) -> Plan {
        let changed_at = self.newest_damage.take().unwrap_or(now);
        self.planned_change = band.map(|_| changed_at);
        let Some((y0, y1, changed)) = band else {
            return Plan {
                scale: self.scale,
                sharp_qp: None,
            };
        };
        let big = changed * 2 > self.soft.len();
        let rows = self.soft.len();
        self.soft_minimum = self.soft_minimum.max(changed.min(rows));
        let (y0, y1) = (y0.min(rows), y1.min(rows));
        if changed == y1 - y0 {
            self.soft[y0..y1].fill(true);
        } else if y0 < y1 {
            // Sparse bands hide the interior positions. Only their edge rows are known;
            // the changed-row count is a lower bound, never the entire enclosing span.
            self.soft[y0] = true;
            self.soft[y1 - 1] = true;
        }
        self.last_change = changed_at;
        if big {
            self.last_big = changed_at;
            self.calm_sharp = false;
        }
        if !self.calm_sharp
            && let Some(plan) = self.settled(now, self.last_big)
        {
            if Some(self.link.at) == self.ladder.half && !self.full_return_fits(now) {
                return Plan {
                    scale: self.scale,
                    sharp_qp: None,
                };
            }
            return plan;
        }
        let scale = if big || self.scale == Scale::Half {
            self.motion_scale()
        } else {
            Scale::Full
        };
        Plan {
            scale,
            sharp_qp: None,
        }
    }

    /// Keep a stale half frame from occupying the wire when the full still is due.
    pub fn defer_half(&self, now: Instant, plan: Plan) -> bool {
        if self.scale != Scale::Half
            || plan.scale != Scale::Half
            || now.saturating_duration_since(self.last_change)
                < Duration::from_secs_f32(1.0 / self.fps())
        {
            return false;
        }
        let Some((at, seconds_per_byte)) = self.last_delivery else {
            return false;
        };
        if now.saturating_duration_since(at) > CAPACITY_WINDOW {
            return false;
        }
        let due = self.full_size_deadline(now);
        now < due
            && self.estimate(self.crf()).is_some_and(|bytes| {
                f64::from(bytes) * 0.25 * seconds_per_byte
                    > due.saturating_duration_since(now).as_secs_f64()
            })
    }

    /// Next useful wakeup, including when the rate that ruled out sharpening expires.
    pub fn deadline(&self, now: Instant) -> Option<Instant> {
        self.deadline_since(now, self.last_change)
    }

    fn needs_sharp(&self) -> bool {
        self.soft
            .iter()
            .filter(|&&soft| soft)
            .count()
            .max(self.soft_minimum)
            * 4
            >= self.soft.len()
    }

    fn full_size_after(&self) -> Duration {
        if self.scale == Scale::Half && self.encoded_cpu_half {
            CPU_FULL_SIZE_AFTER
        } else {
            FULL_SIZE_AFTER
        }
    }

    fn deadline_since(&self, now: Instant, since: Instant) -> Option<Instant> {
        if self.scale == Scale::Half {
            return Some(self.full_size_deadline(now));
        }
        self.sharp_deadline_since(now, since)
    }

    fn full_size_deadline(&self, now: Instant) -> Instant {
        if Some(self.link.at) == self.ladder.half && !self.full_return_fits(now) {
            self.last_change + FULL_SIZE_AFTER
        } else {
            self.last_big + self.full_size_after()
        }
    }

    fn sharp_deadline_since(&self, now: Instant, since: Instant) -> Option<Instant> {
        if !self.needs_sharp() {
            return None;
        }
        let pause = if self.scale == Scale::Half {
            self.full_size_after()
        } else {
            self.cfg.sharpen_after
        };
        let due = since + pause;
        let Some((_, transfer)) = self.sharp_budget(now) else {
            let retry = self
                .sharp_congestion_expiry(now)
                .or_else(|| capacity_expiry(self.capacity.back()?.0, now))?;
            return Some(due.max(retry));
        };
        let quiet = if transfer > Duration::from_millis(150) {
            self.last_input.map(|at| at + Duration::from_secs(1))
        } else {
            None
        };
        Some(quiet.map_or(due, |quiet| due.max(quiet)))
    }

    pub fn still(&self, now: Instant) -> Option<Plan> {
        self.settled(now, self.last_change)
    }

    fn settled(&self, now: Instant, since: Instant) -> Option<Plan> {
        if now < self.deadline_since(now, since)? {
            return None;
        }
        let sharp_qp = self
            .sharp_deadline_since(now, since)
            .filter(|due| now >= *due)
            .and_then(|_| self.sharp_budget(now))
            .map(|(qp, _)| qp);
        Some(Plan {
            scale: Scale::Full,
            sharp_qp,
        })
    }

    fn estimate(&self, qp: f32) -> Option<f32> {
        let idr = self.idr.as_ref()?;
        Some(
            idr.bytes as f32
                * 2.0_f32.powf((idr.qp - qp) / 6.0)
                * (self.width * self.soft.len()) as f32
                / idr.pixels as f32,
        )
    }

    fn return_capacity_kbps(&self, now: Instant) -> Option<f32> {
        let probe = self
            .last_probe
            .filter(|(at, _)| now.saturating_duration_since(*at) <= CAPACITY_WINDOW);
        if self.clear_ticks_needed == 0 {
            return probe.map(|(_, kbps)| kbps);
        }
        // A burst can outrun sustained delivery. If half frames are too small to
        // probe, keep pricing the return from the last observation during the jam.
        let (_, kbps) = probe.or_else(|| self.capacity.back().copied())?;
        Some(
            self.last_congested_kbps
                .map_or(kbps, |delivered| delivered.min(kbps)),
        )
    }

    /// A motion return reopens the full encoder and puts an IDR on the wire.
    fn full_return_fits(&self, now: Instant) -> bool {
        let cap_crf = self.cfg.base_crf + self.ladder.rungs[self.ladder.cap].crf_add;
        self.estimate(cap_crf)
            .zip(self.return_capacity_kbps(now))
            .is_none_or(|(bytes, kbps)| bytes * 8.0 <= 300.0 * kbps)
    }

    fn sharp_congestion_expiry(&self, now: Instant) -> Option<Instant> {
        if self.clear_ticks_needed == 0 {
            return None;
        }
        capacity_expiry(self.last_congested?.0, now)
    }

    fn sharp_budget(&self, now: Instant) -> Option<(u8, Duration)> {
        let cap = if self.sharp_congestion_expiry(now).is_some() {
            self.return_capacity_kbps(now)
        } else {
            self.capacity_kbps(now)
        }
        .unwrap_or(8000.0);
        for qp in self.cfg.sharpen_qp..=51 {
            if f32::from(qp) >= self.crf() - 2.0 {
                return None;
            }
            let seconds = self
                .estimate(f32::from(qp))
                .map_or(0.0, |bytes| bytes * 8.0 / (cap * 1000.0));
            if seconds <= 0.3 {
                return Some((qp, Duration::from_secs_f32(seconds)));
            }
        }
        None
    }

    pub fn sharp_qp(&self, now: Instant) -> Option<u8> {
        self.sharp_budget(now).map(|(qp, _)| qp)
    }

    pub fn encoded(&mut self, now: Instant, frame: Encoded) {
        let changed_at = self.planned_change.take().unwrap_or(now);
        let switched = self.scale != frame.plan.scale;
        let soft_key = frame.key && (frame.owner_key || switched || self.idr.is_none());
        self.scale = frame.plan.scale;
        if self.scale == Scale::Half {
            self.encoded_cpu_half = self.ladder.rungs[self.cpu.at].scale == Scale::Half;
            self.cpu_full_calm_since = None;
        } else if self.cpu_return_pending {
            // Start the failure window when full frames actually resume.
            self.cpu_return_pending = false;
            self.cpu_returned_at = Some(now);
            self.cpu_full_calm_since = Some(now);
        }
        if frame.key && frame.pixels > 0 {
            self.idr = Some(Idr {
                bytes: frame.bytes,
                qp: frame.plan.sharp_qp.map_or(self.crf(), f32::from),
                pixels: frame.pixels,
            });
        }
        if frame.plan.sharp_qp.is_some() {
            self.soft.fill(false);
            self.soft_minimum = 0;
            self.calm_sharp = true;
        } else if soft_key {
            self.soft.fill(true);
            self.last_big = if frame.big { changed_at } else { now };
            self.last_change = self.last_big;
            self.calm_sharp = false;
        }
        if frame.big && !frame.key && frame.plan.scale == Scale::Full {
            self.full_big += 1;
            self.full_big_bytes += frame.bytes as u64;
        }
        if frame.big && !frame.key && !switched {
            self.work.push(frame.work.total());
            self.full_work.push(frame.work.full(frame.plan.scale));
            self.sampled_full += usize::from(frame.plan.scale == Scale::Full);
        }
    }

    pub fn probe(&mut self, now: Instant, bytes: usize, delay: Duration, base_rtt: Duration) {
        if bytes >= 32 * 1024 && !delay.is_zero() {
            // Include RTT: this budgets the whole wait for the next key's ACK.
            self.last_delivery = Some((now, delay.as_secs_f64() / bytes as f64));
        }
        if bytes >= 32 * 1024
            && let Some(transfer) = delay.checked_sub(base_rtt)
            && !transfer.is_zero()
        {
            let sample = (now, bytes as f32 * 8.0 / 1000.0 / transfer.as_secs_f32());
            self.capacity.push_back(sample);
            self.last_probe = Some(sample);
        }
    }

    pub fn capacity_kbps(&self, now: Instant) -> Option<f32> {
        self.capacity
            .iter()
            .filter(|(at, _)| now.saturating_duration_since(*at) <= CAPACITY_WINDOW)
            .map(|(_, kbps)| *kbps)
            .reduce(f32::max)
    }

    fn mark_congested(&mut self, now: Instant, delivered_kbps: f32) {
        self.last_congested = Some((now, delivered_kbps));
        self.clear_ticks_needed = 2;
    }

    /// Price the forced key immediately. The next complete interval measures this stall.
    pub fn window_reset(&mut self, now: Instant) -> Option<Move> {
        self.reset_since_tick = true;
        self.mark_congested(now, 0.0);
        self.link_pressure(now, Why::Reset)
            .map(|(to, why)| self.apply(now, Source::Link, to, why))
    }

    pub fn tick(&mut self, now: Instant, link: Link, cpu: Cpu) -> Vec<Move> {
        let fps = self.fps();
        let work = percentile75(&mut self.work);
        self.work_p75_ms = work.unwrap_or_default().as_secs_f32() * 1000.0;
        let measured = work.filter(|_| self.work.len() >= 5);
        let full_observed = measured.is_some() && self.sampled_full == self.work.len();
        if self.full_work.len() >= 5 {
            self.predicted_full = percentile75(&mut self.full_work);
        }
        let mean_bytes = self.full_big_bytes as f32 / self.full_big.max(1) as f32;
        if self.full_big >= 5 {
            let demand = mean_bytes * 8.0 * fps / 1000.0;
            self.demand_kbps = Some(
                self.demand_kbps
                    .map_or(demand, |old| 0.75 * old + 0.25 * demand),
            );
        }
        let reset = std::mem::take(&mut self.reset_since_tick);
        let congested = reset || link_full(link.blocked);
        // A queue built below the cap hasn't tested what the quality steps can save.
        let late_at_cap = self.link.at == self.ladder.cap
            && !reset
            && congested
            && link.delivered_kbps > 0.0
            && link.queue_ms > 300;
        let severe = congested && self.severe(now, &link, mean_bytes, fps, late_at_cap);
        self.previous_late_at_cap = late_at_cap;
        self.previous_congested_no_ack = congested && link.delivered_kbps == 0.0;
        self.facts = Facts {
            blocked: link.blocked,
            queue_ms: link.queue_ms,
            offered_kbps: self.demand_kbps.unwrap_or(link.sent_kbps),
            delivered_kbps: link.delivered_kbps,
            work_ms: self.work_p75_ms,
            stolen: cpu.stolen,
            frames: link.frames,
            grabs: link.grabs,
            busy: link.busy,
        };
        self.observe_link(now, &link, congested);
        let mut moves = Vec::new();
        if reset {
            // The reset already cast this interval's link vote.
            self.queue_high = link.queue_ms > 150;
        } else if let Some((to, why)) = self.link_step(now, &link, fps, congested, severe) {
            moves.push(self.apply(now, Source::Link, to, why));
        }
        if self.cfg.cpu_guard {
            let work_pressure = measured.is_some_and(|w| w > starved_work(fps));
            let active = link.grabs > 0 || link.frames > 0 || !self.work.is_empty();
            // Quota bursts and steal only explain lost cadence while the pacer could accept
            // damage. Slow measured work is direct evidence even during a shorter interval.
            let quota = cpu.stolen > 0.20 || cpu.throttled;
            let busy = link.damage_pending && link.busy >= Duration::from_millis(500);
            let missed =
                busy && f64::from(link.grabs) < 0.8 * f64::from(fps) * link.busy.as_secs_f64();
            let starved = if work_pressure {
                Some(Why::Work)
            } else if missed && cpu.stolen > 0.20 {
                Some(Why::Steal)
            } else if missed && cpu.throttled {
                Some(Why::Throttled)
            } else {
                None
            };
            let step = if let Some(why) = starved {
                self.cpu_pressure(now, why, measured, full_observed)
            } else if active && (!quota || busy) {
                self.cpu_recovery(now, measured, full_observed)
            } else {
                None
            };
            if let Some((to, why)) = step {
                moves.push(self.apply(now, Source::Cpu, to, why));
            }
        }
        self.full_big = 0;
        self.full_big_bytes = 0;
        self.sampled_full = 0;
        self.work.clear();
        self.full_work.clear();
        moves
    }

    fn apply(&mut self, now: Instant, source: Source, to: usize, why: Why) -> Move {
        let level = match source {
            Source::Link => &mut self.link,
            Source::Cpu => &mut self.cpu,
        };
        let from = level.at;
        level.move_to(to, now);
        if source == Source::Link && why == Why::Reset && to == self.ladder.cap {
            level.pressure = 1;
        }
        Move {
            source,
            from,
            to,
            why,
            facts: self.facts,
            base_crf: self.cfg.base_crf,
            rung: Rung {
                fps: self.fps(),
                crf_add: self.ladder.rungs[self.link.at].crf_add,
                scale: self.motion_scale(),
            },
        }
    }

    /// Price a full motion frame at the quality cap. A keyframe burst alone says
    /// nothing about the cost of continued motion.
    fn severe(
        &self,
        now: Instant,
        link: &Link,
        mean_bytes: f32,
        fps: f32,
        late_at_cap: bool,
    ) -> bool {
        if link.delivered_kbps == 0.0 {
            return self.previous_congested_no_ack && link.sent_kbps > 0.0;
        }
        if late_at_cap && self.previous_late_at_cap {
            return true;
        }
        if self.full_big < 3 {
            return false;
        }
        let cap = self.ladder.rungs[self.ladder.cap];
        let remaining = cap.crf_add - self.ladder.rungs[self.link.at].crf_add;
        let cap_bytes = mean_bytes * 2.0_f32.powf(-remaining / 6.0);
        // Slower scroll frames carry more new rows, so bytes per second stay roughly
        // constant. Pricing high-fps frames at the floor fps underestimates demand.
        let cap_demand_kbps = cap_bytes * 8.0 * fps / 1000.0;
        // A swipe can start mid-tick. Cover one delivery dip with the preceding
        // congested tick's actual rate, so a sustained slowdown still counts.
        let available_kbps = self
            .last_congested
            .filter(|&(at, previous)| {
                now.saturating_duration_since(at) <= Duration::from_secs(3)
                    && link.delivered_kbps < 0.5 * previous
            })
            .map_or(link.delivered_kbps, |(_, previous)| previous);
        cap_bytes * 8.0 > 300.0 * link.delivered_kbps || cap_demand_kbps > 1.5 * available_kbps
    }

    fn observe_link(&mut self, now: Instant, link: &Link, congested: bool) {
        // The motion IDR budget can still use the last observation after the window expires.
        while self.capacity.len() > 1
            && self
                .capacity
                .front()
                .is_some_and(|(at, _)| now.saturating_duration_since(*at) > CAPACITY_WINDOW)
        {
            self.capacity.pop_front();
        }
        if congested {
            self.mark_congested(now, link.delivered_kbps);
            if link.delivered_kbps > 0.0 {
                self.capacity.push_back((now, link.delivered_kbps));
                self.last_congested_kbps = Some(link.delivered_kbps);
            }
            if link.delivered_kbps > 100.0 {
                self.link_kbps = ((link.delivered_kbps * 0.85) as u32).max(300);
            }
        } else {
            if self.clear_ticks_needed > 0 {
                let drained =
                    link.delivered_kbps > 0.0 && link.base_queue_ms.is_some_and(|ms| ms < 40);
                self.clear_ticks_needed = if !drained {
                    2
                } else if self.last_congested.is_some_and(|(at, _)| {
                    now.saturating_duration_since(at) >= STALE_CAPACITY_AFTER_JAM
                }) {
                    0
                } else {
                    self.clear_ticks_needed - 1
                };
            }
            self.link_kbps += self.link_kbps / 10;
            if self
                .last_congested
                .is_none_or(|(at, _)| now.saturating_duration_since(at) >= Duration::from_secs(30))
            {
                self.backoff = Duration::from_secs(2);
            }
        }
        self.link_kbps = self.link_kbps.min(8000);
    }

    fn link_step(
        &mut self,
        now: Instant,
        link: &Link,
        fps: f32,
        congested: bool,
        severe: bool,
    ) -> Option<(usize, Why)> {
        let at = self.link.at;
        let cap = self.ladder.cap;
        if self
            .last_pressure
            .is_some_and(|t| now.saturating_duration_since(t) >= Duration::from_secs(30))
        {
            self.link.need = 3;
        }
        // Returning to full has its own capacity and time test, even on a still screen.
        if Some(at) == self.ladder.half
            && !congested
            && now.saturating_duration_since(self.link.since) >= self.backoff
            && self
                .last_congested
                .is_none_or(|(at, _)| now.saturating_duration_since(at) >= Duration::from_secs(2))
            && self
                .return_capacity_kbps(now)
                .zip(self.demand_kbps)
                .is_none_or(|(cap, demand)| cap >= 1.2 * demand)
            && self.full_return_fits(now)
        {
            self.last_full = Some(now);
            return Some((cap, Why::Capacity));
        }
        if !congested
            && link.frames == 0
            && link.sent_kbps == 0.0
            && link.delivered_kbps == 0.0
            && link.queue_ms == 0
        {
            return None;
        }
        let queued = self.queue_high && link.queue_ms > 150;
        self.queue_high = link.queue_ms > 150;
        if congested || queued {
            let why = if severe {
                Why::Severe
            } else if congested {
                Why::Congested
            } else {
                Why::Queue
            };
            return self.link_pressure(now, why);
        }
        self.link.pressure = 0;
        // Small updates can drain on a link that would still queue a swipe.
        let min_motion_frames = ((fps / 3.0) as usize).clamp(1, 5);
        if at == 0
            || at > cap
            || link.queue_ms >= 40
            || link.delivered_kbps == 0.0
            || self.work.len() < min_motion_frames
        {
            return None;
        }
        self.link.clean = self.link.clean.saturating_add(1);
        let fps_step = at <= self.ladder.floor;
        let needed = if fps_step { self.link.need } else { 2 };
        if self.link.clean < needed {
            return None;
        }
        if fps_step {
            self.link.last_up = Some(now);
        }
        Some((at - 1, Why::Clean))
    }

    fn link_pressure(&mut self, now: Instant, why: Why) -> Option<(usize, Why)> {
        let repeat_reset = why == Why::Reset && recent(self.last_reset.replace(now), now);
        self.link.under_pressure(now);
        self.last_pressure = Some(now);
        let at = self.link.at;
        let cap = self.ladder.cap;
        if at > cap {
            return None;
        }
        // A CPU vote may already have paid for some or all of the fps steps.
        let effective = at.max(self.cpu.at.min(self.ladder.floor));
        if at < cap
            && matches!(why, Why::Reset | Why::Severe)
            && !(repeat_reset && self.ladder.half.is_some())
        {
            return Some((cap, why));
        }
        if repeat_reset || (why != Why::Queue && effective == cap) {
            self.link.pressure = self.link.pressure.saturating_add(1);
            if let Some(half) = self.ladder.half
                && (why == Why::Severe
                    || repeat_reset
                    || self.link.pressure >= 3
                    || recent(self.last_full, now))
            {
                if recent(self.last_full, now) {
                    self.backoff = (self.backoff * 2).min(Duration::from_secs(16));
                }
                let why = if why == Why::Congested { Why::Jam } else { why };
                return Some((half, why));
            }
        } else {
            self.link.pressure = 0;
        }
        let to = (effective + 1).min(cap);
        (to != at).then_some((to, why))
    }

    fn cpu_pressure(
        &mut self,
        now: Instant,
        why: Why,
        work: Option<Duration>,
        full_observed: bool,
    ) -> Option<(usize, Why)> {
        if now.saturating_duration_since(self.last_starved) >= Duration::from_secs(60) {
            self.cpu.need = 3;
        }
        self.last_starved = now;
        self.cpu_full_calm_since = None;
        self.cpu.under_pressure(now);
        let recent_return = recent(self.cpu_returned_at, now);
        if recent_return && full_observed {
            self.cpu_full_floor = work.map(|cost| (now, cost));
        }
        let at = self.cpu.at;
        let floor = self.ladder.floor;
        self.cpu.pressure = self.cpu.pressure.saturating_add(1);
        if at == floor {
            if self.cpu.pressure < 3 && !(self.cpu_trial && full_observed) {
                return None;
            }
            let to = self.ladder.half.or(self.ladder.low)?;
            if self.ladder.half.is_some() && recent_return {
                self.cpu_calm_needed = (self.cpu_calm_needed * 2).min(60);
            }
            self.cpu_trial = false;
            self.cpu_return_pending = false;
            return Some((to, why));
        }
        if self.cpu.pressure < 2 {
            return None;
        }
        if at < floor {
            let to = if let Some(cost) = work.filter(|_| why == Why::Work) {
                (at + 1..=floor)
                    .find(|&i| cost <= starved_work(self.ladder.rungs[i].fps))
                    .unwrap_or(floor)
            } else {
                at + 1
            };
            return Some((to, why));
        }
        self.ladder
            .low
            .filter(|&low| low != at)
            .map(|low| (low, why))
    }

    fn cpu_recovery(
        &mut self,
        now: Instant,
        work: Option<Duration>,
        full_observed: bool,
    ) -> Option<(usize, Why)> {
        let at = self.cpu.at;
        let floor = self.ladder.floor;
        self.cpu.pressure = 0;
        if full_observed {
            self.cpu_trial = false;
        }
        if now.saturating_duration_since(self.last_starved) >= Duration::from_secs(60) {
            self.cpu.need = 3;
        }
        if self.scale == Scale::Full && at <= floor && !self.cpu_return_pending {
            let since = *self.cpu_full_calm_since.get_or_insert(now);
            if now.saturating_duration_since(since) >= Duration::from_secs(60) {
                self.cpu_calm_needed = 3;
                self.cpu_full_floor = None;
            }
        } else {
            self.cpu_full_calm_since = None;
        }
        if at == 0 {
            return None;
        }
        let (above, need) = if Some(at) == self.ladder.low {
            (self.ladder.half.unwrap_or(floor), 3)
        } else if Some(at) == self.ladder.half {
            (floor, self.cpu_calm_needed)
        } else {
            (at - 1, self.cpu.need)
        };
        let budget = full_work(self.ladder.rungs[above].fps);
        if work.is_some_and(|cost| cost > budget) {
            self.cpu.clean = 0;
            return None;
        }
        self.cpu.clean = self.cpu.clean.saturating_add(1);
        if self.cpu.clean < need {
            return None;
        }
        let mut why = Why::Calm;
        if Some(at) == self.ladder.half {
            if self
                .cpu_full_prediction(now)
                .is_none_or(|cost| cost > budget)
            {
                let trial_wait = Duration::from_secs(u64::from(self.cpu_calm_needed) * 10)
                    .min(Duration::from_secs(60));
                if now.saturating_duration_since(self.last_starved) < trial_wait {
                    return None;
                }
                self.cpu_trial = true;
                why = Why::Trial;
            }
            self.cpu_return_pending = true;
            self.cpu_returned_at = None;
        } else if at <= floor {
            self.cpu.last_up = Some(now);
        }
        Some((above, why))
    }

    fn cpu_full_prediction(&self, now: Instant) -> Option<Duration> {
        self.predicted_full.map(|model| {
            self.cpu_full_floor.map_or(model, |(at, measured)| {
                // Only the measured excess decays, toward the current model.
                let age = now.saturating_duration_since(at).as_secs_f64();
                model
                    + measured
                        .saturating_sub(model)
                        .mul_f64(2_f64.powf(-age / 30.0))
            })
        })
    }
}

pub fn link_full(blocked: f32) -> bool {
    blocked > 0.25
}

fn percentile75(samples: &mut [Duration]) -> Option<Duration> {
    samples.sort_unstable();
    samples
        .get((samples.len() * 3).div_ceil(4).saturating_sub(1))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    const WIDTH: usize = 780;
    const ROWS: usize = 1688;
    const BIG: Option<(usize, usize, usize)> = Some((0, ROWS, ROWS));

    fn cfg() -> Config {
        Config {
            base_crf: 24.0,
            crf_range: 6.0,
            sharpen_qp: 18,
            sharpen_after: Duration::from_millis(250),
            max_fps: 30.0,
            adaptive_size: true,
            cpu_guard: true,
        }
    }
    fn picture(t: Instant) -> Picture {
        Picture::new(cfg(), WIDTH, ROWS, t)
    }
    fn at(t: Instant, seconds: u64) -> Instant {
        t + Duration::from_secs(seconds)
    }
    fn frame(scale: Scale, work_ms: u64) -> Encoded {
        Encoded {
            plan: Plan {
                scale,
                sharp_qp: None,
            },
            key: false,
            owner_key: true,
            bytes: 40_000,
            pixels: WIDTH * ROWS / if scale == Scale::Half { 4 } else { 1 },
            big: true,
            work: Work {
                grab: Duration::from_millis(2),
                convert: Duration::ZERO,
                encode: Duration::from_millis(work_ms - 2),
            },
        }
    }
    fn motion(p: &mut Picture, t: Instant, scale: Scale, work_ms: u64) {
        // First sample establishes the encoder size; switches must not pollute work.
        for _ in 0..6 {
            p.encoded(t, frame(scale, work_ms));
        }
    }
    fn jam() -> Link {
        Link {
            blocked: 0.6,
            sent_kbps: 9600.0,
            delivered_kbps: 3000.0,
            queue_ms: 250,
            base_queue_ms: Some(250),
            frames: 8,
            grabs: 8,
            damage_pending: true,
            busy: Duration::from_millis(400),
        }
    }
    fn active(frames: u32) -> Link {
        Link {
            frames,
            grabs: frames,
            damage_pending: true,
            busy: Duration::from_secs(1),
            delivered_kbps: 3000.0,
            base_queue_ms: Some(0),
            ..Link::default()
        }
    }
    fn to_link_half(p: &mut Picture, t: Instant) -> Instant {
        to_link_half_with_queue(p, t, jam().queue_ms)
    }
    fn to_link_half_with_queue(p: &mut Picture, t: Instant, queue_ms: u32) -> Instant {
        for n in 1..=8 {
            p.tick(at(t, n), Link { queue_ms, ..jam() }, Cpu::default());
        }
        assert_eq!(p.motion_scale(), Scale::Half);
        at(t, 8)
    }
    fn to_cpu_half(p: &mut Picture, t: Instant) -> Instant {
        for n in 1..=5 {
            motion(p, at(t, n), Scale::Full, 70);
            p.tick(at(t, n), active(6), Cpu::default());
            if p.motion_scale() == Scale::Half {
                return at(t, n);
            }
        }
        panic!("sustained 70 ms work must reach half size");
    }
    fn settle(p: &mut Picture, t: Instant) {
        let mut key = frame(Scale::Full, 10);
        key.key = true;
        key.plan.sharp_qp = Some(18);
        key.bytes = 150_000;
        p.encoded(t, key);
    }
    fn send(p: &mut Picture, t: Instant, plan: Plan) {
        let mut f = frame(plan.scale, 10);
        f.plan = plan;
        f.key = plan.sharp_qp.is_some();
        p.encoded(t, f);
    }
    fn capacity(p: &mut Picture, t: Instant, kbps: f32) {
        p.probe(
            t,
            100_000,
            Duration::from_secs_f32(800.0 / kbps) + Duration::from_millis(30),
            Duration::from_millis(30),
        );
    }

    #[test]
    fn ladder_half_frames_preserve_phone_screen_dimensions() {
        use crate::encoder::{Backend, Encoder, Settings};
        use crate::{wire, yuv::I420};

        let start = Instant::now();
        let settings = Settings {
            backend: Backend::default_name().parse().unwrap(),
            preset: "ultrafast".into(),
            threads: 1,
            crf: cfg().base_crf,
        };
        let late_queue = |p: &mut Picture, t| to_link_half_with_queue(p, t, 400);
        // At 2×, the 393×852 phone loses two screen pixels in width, the 430×853 in height.
        for (width, height, coded_width, flags) in
            [(786, 1704, 392_u16, 0x15), (860, 1706, 430, 0x25)]
        {
            let rows = usize::from(height);
            for enter_half in [to_link_half, late_queue, to_cpu_half] {
                let mut p = Picture::new(cfg(), usize::from(width), rows, start);
                let now = enter_half(&mut p, start);
                let plan = p.plan(now, Some((0, rows, rows)));
                let (w, h) = plan.scale.dims(width, height);
                let mut enc = Encoder::open(w, h, &settings, p.crf(), p.link_kbps()).unwrap();
                let pixels = I420::new(usize::from(w), usize::from(h));
                let out = enc.encode(&pixels, true, plan.sharp_qp, 0).unwrap();
                let frame = wire::frame(
                    1,
                    out.key,
                    plan.sharp_qp.is_some(),
                    plan.scale,
                    width,
                    height,
                    &out.data,
                );

                assert_eq!(frame[9], flags);
                assert_eq!(&frame[10..12], &coded_width.to_be_bytes());
                assert_eq!(&frame[12..14], &852_u16.to_be_bytes());
            }
        }
    }

    #[test]
    fn tests_that_fps_rungs_follow_the_configured_max() {
        for (max_fps, expected) in [
            (60.0, &[60.0, 30.0, 20.0, 15.0][..]),
            (30.0, &[30.0, 20.0, 15.0]),
            (24.0, &[24.0, 20.0, 15.0]),
            (19.5, &[19.5, 15.0]),
            (15.0, &[15.0]),
            (10.0, &[10.0]),
        ] {
            let ladder = Ladder::new(Config {
                max_fps,
                crf_range: 5.0,
                ..cfg()
            });
            let rates: Vec<_> = ladder.rungs[..=ladder.floor]
                .iter()
                .map(|r| r.fps)
                .collect();
            assert_eq!(rates, expected);
            let quality: Vec<_> = ladder.rungs[ladder.floor + 1..=ladder.cap]
                .iter()
                .map(|r| r.crf_add)
                .collect();
            assert_eq!(quality, [2.0, 4.0, 5.0]);
            assert_eq!(ladder.half, Some(ladder.cap + 1));
            assert_eq!(ladder.low.is_some(), max_fps > 10.0);
        }
        let fixed = Ladder::new(Config {
            adaptive_size: false,
            crf_range: 0.0,
            ..cfg()
        });
        assert_eq!(fixed.floor, fixed.cap);
        assert_eq!(fixed.half, None);
        assert_eq!(
            fixed.low.map(|low| fixed.rungs[low]),
            Some(Rung {
                fps: 10.0,
                crf_add: 0.0,
                scale: Scale::Full
            })
        );
    }

    #[test]
    fn tests_that_link_pressure_steps_fps_before_quality() {
        let t = Instant::now();
        let mut p = picture(t);
        for (n, (fps, crf)) in [(20.0, 24.0), (15.0, 24.0), (15.0, 26.0)]
            .into_iter()
            .enumerate()
        {
            let now = at(t, n as u64 + 1);
            p.tick(now, jam(), Cpu::default());
            assert_eq!(
                (p.fps(), p.crf(), p.plan(now, BIG).scale),
                (fps, crf, Scale::Full)
            );
        }
    }
    #[test]
    fn tests_that_missing_rate_data_still_steps_fps() {
        let t = Instant::now();
        let mut p = picture(t);
        p.tick(
            at(t, 1),
            Link {
                blocked: 0.8,
                ..Link::default()
            },
            Cpu::default(),
        );
        assert_eq!((p.fps(), p.crf()), (20.0, 24.0));
    }
    #[test]
    fn tests_that_queue_pressure_needs_two_ticks() {
        let t = Instant::now();
        let mut p = picture(t);
        for (n, (queue_ms, fps)) in [(151, 30.0), (40, 30.0), (151, 30.0), (151, 20.0)]
            .into_iter()
            .enumerate()
        {
            p.tick(
                at(t, n as u64 + 1),
                Link {
                    queue_ms,
                    ..active(15)
                },
                Cpu::default(),
            );
            assert_eq!((p.fps(), p.crf()), (fps, 24.0));
        }
    }
    #[test]
    fn tests_that_quality_steps_by_two_and_stops_at_crf_range() {
        let t = Instant::now();
        for (range, expected) in [
            (5.0, [26.0, 28.0, 29.0]),
            (2.5, [26.0, 26.5, 26.5]),
            (0.0, [24.0; 3]),
        ] {
            let mut p = Picture::new(
                Config {
                    crf_range: range,
                    max_fps: 15.0,
                    adaptive_size: false,
                    ..cfg()
                },
                WIDTH,
                ROWS,
                t,
            );
            for (n, crf) in expected.into_iter().enumerate() {
                p.tick(at(t, n as u64 + 1), jam(), Cpu::default());
                assert_eq!((p.crf(), p.motion_scale()), (crf, Scale::Full));
            }
        }
    }

    #[test]
    fn tests_that_half_needs_three_congested_ticks_at_the_cap() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=8 {
            p.tick(at(t, n), jam(), Cpu::default());
            assert_eq!(
                p.motion_scale(),
                if n < 8 { Scale::Full } else { Scale::Half }
            );
            if n >= 5 {
                assert_eq!((p.fps(), p.crf()), (15.0, 30.0));
            }
        }
    }

    #[test]
    fn tests_that_queue_pressure_alone_never_sets_half() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=120 {
            p.tick(
                at(t, n),
                Link {
                    queue_ms: 400,
                    ..active(15)
                },
                Cpu::default(),
            );
            assert_eq!(p.motion_scale(), Scale::Full);
        }
        assert_eq!((p.fps(), p.crf()), (15.0, 30.0));
    }

    fn sized_motion(p: &mut Picture, now: Instant, bytes: usize, count: usize) {
        for _ in 0..count {
            let mut f = frame(Scale::Full, 10);
            f.bytes = bytes;
            p.encoded(now, f);
        }
    }

    #[test]
    fn tests_that_a_severe_jam_reaches_half_on_the_second_tick() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=2 {
            sized_motion(&mut p, at(t, n), 240_000, 3);
            let moves = p.tick(at(t, n), jam(), Cpu::default());
            assert_eq!(moves[0].why, Why::Severe);
            assert_eq!((p.fps(), p.crf()), (15.0, 30.0));
            assert_eq!(
                p.motion_scale(),
                if n == 1 { Scale::Full } else { Scale::Half }
            );
        }
    }

    #[test]
    fn tests_that_a_stream_too_big_for_any_full_rung_goes_half_in_two_ticks() {
        let t = Instant::now();
        let mut p = Picture::new(
            Config {
                max_fps: 15.0,
                ..cfg()
            },
            WIDTH,
            ROWS,
            t,
        );
        // At the cap each frame takes only 147 ms, but 15 per second need 6.6 Mb/s.
        for (n, bytes, scale) in [(1, 110_000, Scale::Full), (2, 55_000, Scale::Half)] {
            let now = at(t, n);
            sized_motion(&mut p, now, bytes, 3);
            let moves = p.tick(now, jam(), Cpu::default());
            assert_eq!(moves.first().map(|m| m.why), Some(Why::Severe));
            assert_eq!(
                (p.fps(), p.crf(), p.plan(now, BIG).scale),
                (15.0, 30.0, scale)
            );
        }
    }

    #[test]
    fn tests_that_the_rate_test_uses_three_frames_and_the_mean_at_the_paced_fps() {
        let t = Instant::now();
        // At 30 fps, a 37.5 kB mean needs exactly 1.5 times the 3 Mb/s delivery at the cap.
        for (samples, why) in [
            (&[110_000, 110_000][..], Why::Congested),
            (&[25_000, 37_500, 50_000], Why::Congested),
            (&[25_000, 37_500, 50_003], Why::Severe),
        ] {
            let mut p = picture(t);
            capacity(&mut p, t, 12_000.0);
            for &bytes in samples {
                sized_motion(&mut p, t, bytes, 1);
            }
            let moves = p.tick(at(t, 1), jam(), Cpu::default());
            assert_eq!(moves.first().map(|m| m.why), Some(why), "{samples:?}");
        }
    }

    #[test]
    fn tests_that_only_a_recent_congested_tick_covers_a_delivery_dip() {
        let t = Instant::now();
        // At 20 fps, 35 kB frames need 2.8 Mb/s at the cap.
        for (age_ms, delivered_kbps, bytes, why) in [
            (3000, 1499.0, 35_000, Why::Congested),
            (3001, 1499.0, 35_000, Why::Severe),
            (1000, 1500.0, 35_000, Why::Severe),
            // One 20 kB frame at the cap takes 320 ms at this tick's delivery.
            (1000, 500.0, 40_000, Why::Severe),
        ] {
            let mut p = picture(t);
            p.tick(t, jam(), Cpu::default());
            p.tick(
                t + Duration::from_millis(500),
                Link {
                    delivered_kbps: 750.0,
                    ..active(20)
                },
                Cpu::default(),
            );
            let now = t + Duration::from_millis(age_ms);
            sized_motion(&mut p, now, bytes, 3);
            let moves = p.tick(
                now,
                Link {
                    delivered_kbps,
                    ..jam()
                },
                Cpu::default(),
            );
            assert_eq!(
                moves.first().map(|m| m.why),
                Some(why),
                "{age_ms} ms since congestion, {delivered_kbps} kb/s delivered"
            );
        }
    }

    #[test]
    fn tests_that_a_second_slow_tick_uses_the_previous_actual_delivery() {
        let t = Instant::now();
        for first_dip in [0.0, 500.0] {
            let mut p = picture(t);
            p.tick(t, jam(), Cpu::default());
            for (n, delivered_kbps, why) in
                [(1, first_dip, Why::Congested), (2, 500.0, Why::Severe)]
            {
                let now = at(t, n);
                sized_motion(&mut p, now, 20_000, 3);
                let moves = p.tick(
                    now,
                    Link {
                        delivered_kbps,
                        ..jam()
                    },
                    Cpu::default(),
                );
                assert_eq!(moves.first().map(|m| m.why), Some(why), "tick {n}");
            }
        }
    }

    #[test]
    fn tests_that_stalled_delivery_needs_two_consecutive_congested_ticks() {
        let t = Instant::now();
        let mut p = picture(t);
        for (n, why, fps, crf, scale) in [
            (1, Why::Congested, 20.0, 24.0, Scale::Full),
            (2, Why::Severe, 15.0, 30.0, Scale::Full),
            (3, Why::Severe, 15.0, 30.0, Scale::Half),
        ] {
            let moves = p.tick(
                at(t, n),
                Link {
                    delivered_kbps: 0.0,
                    ..jam()
                },
                Cpu::default(),
            );
            assert_eq!(moves.first().map(|m| m.why), Some(why));
            assert_eq!((p.fps(), p.crf(), p.motion_scale()), (fps, crf, scale));
        }
    }

    #[test]
    fn tests_that_an_isolated_no_ack_second_at_the_cap_keeps_full_size() {
        let t = Instant::now();
        for intervening in [active(15), Link::default()] {
            let mut p = picture(t);
            sized_motion(&mut p, t, 240_000, 3);
            p.tick(t, jam(), Cpu::default());
            let stalled = || Link {
                delivered_kbps: 0.0,
                ..jam()
            };
            assert!(p.tick(at(t, 1), stalled(), Cpu::default()).is_empty());
            p.tick(at(t, 2), intervening, Cpu::default());
            assert!(p.tick(at(t, 3), stalled(), Cpu::default()).is_empty());
            assert_eq!(p.motion_scale(), Scale::Full);
            let moves = p.tick(at(t, 4), stalled(), Cpu::default());
            assert_eq!(moves.first().map(|m| m.why), Some(Why::Severe));
            assert_eq!(p.motion_scale(), Scale::Half);
        }
    }

    #[test]
    fn tests_that_a_late_queue_below_the_cap_takes_quality_steps() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=2 {
            p.tick(at(t, n), jam(), Cpu::default());
        }
        assert_eq!((p.fps(), p.crf()), (15.0, 24.0));
        for (n, queue_ms, crf, scale, why) in [
            (3, 402, 26.0, Scale::Full, Some(Why::Congested)),
            (4, 500, 28.0, Scale::Full, Some(Why::Congested)),
            (5, 596, 30.0, Scale::Full, Some(Why::Congested)),
            (6, 596, 30.0, Scale::Full, None),
            (7, 596, 30.0, Scale::Half, Some(Why::Severe)),
        ] {
            let moves = p.tick(at(t, n), Link { queue_ms, ..jam() }, Cpu::default());
            assert_eq!((p.crf(), p.motion_scale()), (crf, scale), "tick {n}");
            assert_eq!(moves.first().map(|m| m.why), why);
        }
    }

    #[test]
    fn tests_that_only_consecutive_late_congested_acks_at_the_cap_are_severe() {
        let t = Instant::now();
        for (queue_ms, delivered_kbps, blocked, second, third) in [
            (300, 3000.0, 0.6, None, Some(Why::Jam)),
            (301, 3000.0, 0.6, Some(Why::Severe), None),
            (301, 0.0, 0.6, None, Some(Why::Jam)),
            (301, 3000.0, 0.1, None, None),
            (0, 0.0, 0.0, None, None),
        ] {
            let mut p = picture(t);
            sized_motion(&mut p, t, 240_000, 3);
            p.tick(t, jam(), Cpu::default());
            let late = || Link {
                queue_ms: 301,
                ..jam()
            };
            assert!(p.tick(at(t, 1), late(), Cpu::default()).is_empty());
            let moves = p.tick(
                at(t, 2),
                Link {
                    queue_ms,
                    delivered_kbps,
                    blocked,
                    ..Link::default()
                },
                Cpu::default(),
            );
            assert_eq!(moves.first().map(|m| m.why), second);
            let moves = p.tick(at(t, 3), late(), Cpu::default());
            assert_eq!(moves.first().map(|m| m.why), third);
        }
    }

    fn costly_half_key(p: &mut Picture, t: Instant) {
        let mut f = frame(Scale::Half, 10);
        f.key = true;
        f.bytes = 80_000; // A full IDR at the same QP is 320 kB, or 853 ms on 3 Mb/s.
        p.encoded(t, f);
    }

    #[test]
    fn tests_that_a_jammed_link_keeps_half_while_small_damage_continues() {
        let t = Instant::now();
        let mut p = picture(t);
        let half = to_link_half(&mut p, t);
        costly_half_key(&mut p, half);
        for n in 1..=60 {
            let now = at(half, n);
            let congested = n % 6 == 0;
            p.tick(
                now,
                Link {
                    blocked: if congested { 0.4 } else { 0.0 },
                    delivered_kbps: if congested { 0.0 } else { 3000.0 },
                    base_queue_ms: Some(150),
                    ..active(15)
                },
                Cpu::default(),
            );
            let plan = p.plan(now, Some((10, 26, 16)));
            assert_eq!(
                plan,
                Plan {
                    scale: Scale::Half,
                    sharp_qp: None
                }
            );
            send(&mut p, now, plan);
        }
        assert_eq!(p.capacity_kbps(at(half, 60)), None);
        assert!(
            !p.full_return_fits(at(half, 60)),
            "the last known link still prices the IDR"
        );
    }

    #[test]
    fn tests_that_releasing_jam_caps_needs_consecutive_near_base_acks_with_delivery() {
        let t = Instant::now();
        let mut p = picture(t);
        let half = to_link_half(&mut p, t);
        costly_half_key(&mut p, half);
        let ticks = [
            (Some(39), 3000.0),
            (Some(40), 3000.0),
            (Some(39), 3000.0),
            (None, 3000.0),
            (Some(39), 3000.0),
            (Some(0), 0.0),
            (Some(39), 3000.0),
            (Some(39), 3000.0),
        ];
        for (i, (base_queue_ms, delivered_kbps)) in ticks.into_iter().enumerate() {
            let now = at(half, i as u64 + 1);
            let link = Link {
                base_queue_ms,
                delivered_kbps,
                ..active(15)
            };
            p.tick(now, link, Cpu::default());
            let plan = p.plan(now, Some((10, 26, 16)));
            assert_eq!(
                plan.scale,
                if i + 1 == ticks.len() {
                    Scale::Full
                } else {
                    Scale::Half
                },
                "after tick {}",
                i + 1
            );
            send(&mut p, now, plan);
        }
        let cleared = at(half, ticks.len() as u64);
        assert_eq!(p.capacity_kbps(cleared), Some(3000.0));
        assert_eq!(
            p.return_capacity_kbps(cleared),
            None,
            "jam delivery is not a probe"
        );
    }

    #[test]
    fn tests_that_half_returns_use_the_lower_of_probe_and_congested_delivery() {
        for (full_key_bytes, probe, delivery) in [
            (330_000, 8000.0, 3500.0),
            (240_000, 8000.0, 3500.0),
            (240_000, 3500.0, 8000.0),
        ] {
            let t = Instant::now();
            let mut p = picture(t);
            let half = to_link_half(&mut p, t);
            let mut key = frame(Scale::Half, 10);
            key.key = true;
            key.bytes = full_key_bytes / 4;
            p.encoded(half, key);
            let jammed = at(half, 2);
            for (now, delivered_kbps) in [(at(half, 1), 12_000.0), (jammed, delivery)] {
                p.tick(
                    now,
                    Link {
                        delivered_kbps,
                        ..jam()
                    },
                    Cpu::default(),
                );
            }
            capacity(&mut p, at(jammed, 5), probe);
            for seconds in [5, 19] {
                let now = at(jammed, seconds);
                assert!(
                    !p.full_return_fits(now),
                    "{full_key_bytes} B, probe {probe}, delivery {delivery}, +{seconds}s"
                );
                p.tick(
                    now,
                    Link {
                        base_queue_ms: Some(150),
                        ..active(15)
                    },
                    Cpu::default(),
                );
                assert_eq!(p.motion_scale(), Scale::Half);
            }
            let cleared = at(jammed, 20);
            p.tick(cleared, active(15), Cpu::default());
            assert_eq!(p.motion_scale(), Scale::Full);
        }
    }

    #[test]
    fn tests_that_fresh_capacity_always_applies_and_congestion_rearms_the_last_sample() {
        let t = Instant::now();
        let mut p = picture(t);
        let half = to_link_half(&mut p, t);
        costly_half_key(&mut p, half);
        capacity(&mut p, half, 3000.0);
        for n in 1..=2 {
            p.tick(at(half, n), active(15), Cpu::default());
        }
        assert!(
            !p.full_return_fits(at(half, 2)),
            "a fresh sample still limits full-size keys"
        );
        assert!(
            p.full_return_fits(at(half, 11)),
            "the probe expired after the queue drained"
        );
        p.tick(
            at(half, 12),
            Link {
                blocked: 0.4,
                ..Link::default()
            },
            Cpu::default(),
        );
        assert!(
            !p.full_return_fits(at(half, 12)),
            "new congestion restores even the expired estimate"
        );
        capacity(&mut p, at(half, 40), 3000.0);
        assert!(
            !p.full_return_fits(at(half, 42)),
            "fresh probes always apply"
        );
    }

    #[test]
    fn tests_that_an_expensive_full_still_waits_for_a_second_without_small_changes() {
        let t = Instant::now();
        let mut p = picture(t);
        let half = to_link_half(&mut p, t);
        costly_half_key(&mut p, half);
        for n in 1..=30 {
            let now = half + Duration::from_millis(n * 100);
            let plan = p.plan(now, Some((10, 26, 16)));
            assert_eq!(plan.scale, Scale::Half);
            let mut f = frame(plan.scale, 10);
            f.big = false;
            p.encoded(now, f);
            let gap = now + Duration::from_millis(67);
            assert!(p.still(gap).is_none());
            assert_eq!(p.deadline(gap), Some(now + FULL_SIZE_AFTER));
        }
        let due = at(half, 4);
        assert!(p.still(due - Duration::from_millis(1)).is_none());
        assert_eq!(p.still(due).map(|plan| plan.scale), Some(Scale::Full));
    }

    #[test]
    fn tests_that_a_fitting_full_still_keeps_its_deadline_during_small_changes() {
        let t = Instant::now();
        let mut p = picture(t);
        let half = to_link_half(&mut p, t);
        let mut key = frame(Scale::Half, 10);
        key.key = true;
        key.bytes = 24_000; // A 96 kB full key fits the measured 3 Mb/s.
        p.encoded(half, key);
        let now = half + Duration::from_millis(950);
        assert_eq!(p.plan(now, Some((10, 26, 16))).scale, Scale::Half);
        assert_eq!(p.deadline(now), Some(at(half, 1)));
        assert_eq!(
            p.still(at(half, 1)).map(|plan| plan.scale),
            Some(Scale::Full)
        );
    }

    #[test]
    fn tests_that_a_truly_still_screen_still_returns_full_on_a_jam() {
        let t = Instant::now();
        let mut p = picture(t);
        let half = to_link_half(&mut p, t);
        costly_half_key(&mut p, half);
        assert!(!p.full_return_fits(at(half, 1)));
        assert_eq!(
            p.still(at(half, 1)).map(|plan| plan.scale),
            Some(Scale::Full)
        );
    }

    #[test]
    fn tests_that_small_damage_after_a_full_still_never_starts_half_size()
    -> Result<(), &'static str> {
        let t = Instant::now();
        let mut p = picture(t);
        let half = to_link_half(&mut p, t);
        costly_half_key(&mut p, half);
        let quiet = at(half, 1);
        let mut full = frame(Scale::Full, 10);
        full.plan = p
            .still(quiet)
            .ok_or("a still screen must get a full frame")?;
        full.key = true;
        full.big = false;
        full.bytes = 160_000;
        p.encoded(quiet, full);
        let first_ack = at(quiet, 1);
        assert!(
            p.still(first_ack).is_none(),
            "the recent jam limits sharpening"
        );
        // After the queue drains, sharpening can use the fast peak while the
        // latest probe still holds motion at half size.
        let next = at(first_ack, 1);
        for (now, kbps) in [(first_ack, 8000.0), (next, 3000.0)] {
            capacity(&mut p, now, kbps);
            p.tick(now, active(15), Cpu::default());
        }
        assert!(p.sharp_qp(next).is_some());
        assert!(!p.full_return_fits(next));
        assert_eq!(
            p.plan(next, Some((10, 26, 16))),
            Plan {
                scale: Scale::Full,
                sharp_qp: None
            }
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_fresh_probe_restores_full_when_the_congested_rate_expires() {
        let t = Instant::now();
        let mut p = picture(t);
        let half = to_link_half(&mut p, t);
        costly_half_key(&mut p, half);
        capacity(&mut p, at(half, 19), 9000.0);
        p.tick(at(half, 19), active(15), Cpu::default());
        assert_eq!(p.motion_scale(), Scale::Half);
        let moves = p.tick(at(half, 20), active(15), Cpu::default());
        assert_eq!(moves[0].why, Why::Capacity);
        assert_eq!(
            (p.motion_scale(), p.fps(), p.crf()),
            (Scale::Full, 15.0, 30.0)
        );
    }

    #[test]
    fn tests_that_a_severe_tick_needs_three_samples_over_the_transfer_budget() {
        let t = Instant::now();
        for (bytes, count, why) in [
            (240_000, 2, Why::Congested),
            (200_000, 3, Why::Congested),
            (225_000, 3, Why::Congested),
            (225_001, 3, Why::Severe),
        ] {
            // At 4 fps these frames fit the rate budget, isolating the 300 ms limit.
            let mut p = Picture::new(
                Config {
                    max_fps: 4.0,
                    ..cfg()
                },
                WIDTH,
                ROWS,
                t,
            );
            sized_motion(&mut p, t, bytes, count);
            let moves = p.tick(at(t, 1), jam(), Cpu::default());
            assert_eq!(moves[0].why, why);
        }
    }

    #[test]
    fn tests_that_cellular_frames_are_never_severe() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=60 {
            // 30 kB full frames exceed 3 Mb/s at base quality. The first +2 step fits at 15 fps.
            let bytes = (30_000.0 * 2.0_f32.powf((24.0 - p.crf()) / 6.0)) as usize;
            let demand = bytes as f32 * p.fps() * 0.008;
            sized_motion(&mut p, at(t, n), bytes, 6);
            let link = if demand > 3000.0 { jam() } else { active(15) };
            let moves = p.tick(at(t, n), link, Cpu::default());
            assert!(moves.iter().all(|m| m.why != Why::Severe));
            assert_eq!(p.motion_scale(), Scale::Full);
            if n == 1 {
                assert_eq!((p.fps(), p.crf()), (20.0, 24.0));
            }
        }
    }

    #[test]
    fn tests_that_cellular_carousel_frames_never_trip_the_rate_test() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=60 {
            let now = at(t, n);
            // Each swipe backs up the link for three seconds, then the queue drains.
            let swipe = (n - 1) % 15 < 3;
            sized_motion(&mut p, now, if swipe { 15_000 } else { 10_000 }, 6);
            let link = if swipe { jam() } else { active(p.fps() as u32) };
            let moves = p.tick(now, link, Cpu::default());
            assert!(moves.iter().all(|m| m.why != Why::Severe), "tick {n}");
            assert_eq!(p.plan(now, BIG).scale, Scale::Full, "tick {n}");
            if n == 3 {
                assert_eq!((p.fps(), p.crf()), (15.0, 26.0));
            }
        }
    }

    #[test]
    fn tests_that_recovery_runs_size_then_quality_then_fps() {
        let t = Instant::now();
        let mut p = picture(t);
        let half = to_link_half(&mut p, t);
        let mut changes = Vec::new();
        for n in 1..=14 {
            if n > 2 {
                sized_motion(&mut p, at(half, n), 10_000, 5);
            }
            for m in p.tick(at(half, n), active(15), Cpu::default()) {
                assert_eq!(m.to + 1, m.from);
                changes.push((n, p.motion_scale(), p.crf(), p.fps()));
            }
        }
        assert_eq!(
            changes,
            [
                (2, Scale::Full, 30.0, 15.0),
                (4, Scale::Full, 28.0, 15.0),
                (6, Scale::Full, 26.0, 15.0),
                (8, Scale::Full, 24.0, 15.0),
                (11, Scale::Full, 24.0, 20.0),
                (14, Scale::Full, 24.0, 30.0)
            ]
        );
    }

    #[test]
    fn tests_that_quality_recovers_at_two_fps() {
        let t = Instant::now();
        let mut p = Picture::new(
            Config {
                max_fps: 2.0,
                ..cfg()
            },
            WIDTH,
            ROWS,
            t,
        );
        for n in 1..=3 {
            p.tick(
                at(t, n),
                Link {
                    blocked: 0.6,
                    ..active(2)
                },
                Cpu::default(),
            );
        }
        assert_eq!(p.crf(), 30.0);

        for (i, crf) in [30.0, 28.0, 28.0, 26.0, 26.0, 24.0].into_iter().enumerate() {
            let now = at(t, 4 + i as u64);
            sized_motion(&mut p, now, 10_000, 2);
            p.tick(now, active(2), Cpu::default());
            assert_eq!((p.fps(), p.crf()), (2.0, crf));
        }
    }

    #[test]
    fn tests_that_the_motion_gate_is_unchanged_at_fifteen_and_thirty_fps() {
        let t = Instant::now();
        for (max_fps, paced_fps) in [(20.0, 15.0), (60.0, 30.0)] {
            let mut p = Picture::new(Config { max_fps, ..cfg() }, WIDTH, ROWS, t);
            p.tick(t, jam(), Cpu::default());
            assert_eq!(p.fps(), paced_fps);
            for (i, frames) in [4, 4, 4, 5, 5, 5].into_iter().enumerate() {
                let now = at(t, i as u64 + 1);
                sized_motion(&mut p, now, 10_000, frames);
                p.tick(now, active(frames as u32), Cpu::default());
                assert_eq!(p.fps(), if i == 5 { max_fps } else { paced_fps });
            }
        }
    }

    #[test]
    fn tests_that_a_ten_fps_cpu_limit_needs_three_motion_frames_for_link_recovery() {
        let t = Instant::now();
        let mut p = Picture::new(
            Config {
                adaptive_size: false,
                ..cfg()
            },
            WIDTH,
            ROWS,
            t,
        );
        for n in 1..=5 {
            let now = at(t, n);
            motion(&mut p, now, Scale::Full, 70);
            p.tick(now, if n == 5 { jam() } else { active(6) }, Cpu::default());
        }
        assert_eq!((p.fps(), p.crf()), (10.0, 26.0));
        for (i, frames) in [2, 3, 3].into_iter().enumerate() {
            let now = at(t, 6 + i as u64);
            sized_motion(&mut p, now, 10_000, frames);
            p.tick(now, active(10), Cpu::default());
            assert_eq!(p.crf(), if i == 2 { 24.0 } else { 26.0 });
        }
    }

    #[test]
    fn tests_that_small_changes_between_swipes_do_not_climb_fps() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=2 {
            p.tick(at(t, n), jam(), Cpu::default());
        }
        assert_eq!(p.fps(), 15.0);

        for n in 3..=22 {
            let now = at(t, n);
            for i in 0..15 {
                p.encoded(
                    now,
                    Encoded {
                        bytes: 250,
                        big: i < 5,
                        key: i == 4,
                        ..frame(Scale::Full, 10)
                    },
                );
            }
            p.tick(
                now,
                Link {
                    sent_kbps: 30.0,
                    delivered_kbps: 30.0,
                    ..active(15)
                },
                Cpu::default(),
            );
            assert_eq!(p.fps(), 15.0, "carousel tick {n}");
        }
        for n in 23..=25 {
            sized_motion(&mut p, at(t, n), 10_000, 5);
            p.tick(at(t, n), active(15), Cpu::default());
            assert_eq!(p.fps(), if n == 25 { 20.0 } else { 15.0 });
        }
    }

    #[test]
    fn tests_that_alternating_clean_and_jittery_ticks_restore_full_cadence() {
        let t = Instant::now();
        let mut p = picture(t);
        p.window_reset(t);
        p.tick(at(t, 1), Link::default(), Cpu::default());
        assert_eq!((p.crf(), p.fps()), (30.0, 15.0));
        let mut climbs = Vec::new();
        for n in 2..=24 {
            sized_motion(&mut p, at(t, n), 10_000, 5);
            let link = Link {
                queue_ms: if n % 2 == 0 { 30 } else { 50 },
                ..active(15)
            };
            if !p.tick(at(t, n), link, Cpu::default()).is_empty() {
                climbs.push(n);
            }
        }
        assert_eq!(climbs, [4, 8, 12, 18, 24]);
        assert_eq!((p.crf(), p.fps()), (24.0, 30.0));
    }

    #[test]
    fn tests_that_only_pressure_discards_a_clean_link_tick() {
        let t = Instant::now();
        let mut p = picture(t);
        p.window_reset(t);
        p.tick(t, Link::default(), Cpu::default());
        for (n, queue_ms, blocked, big_frames, crf) in [
            (1, 30, 0.0, 5, 30.0),
            (2, 40, 0.0, 5, 30.0),
            (3, 150, 0.0, 5, 30.0),
            (4, 30, 0.4, 5, 30.0),
            (5, 30, 0.0, 5, 30.0),
            (6, 150, 0.0, 5, 30.0),
            (7, 30, 0.0, 4, 30.0),
            (8, 30, 0.0, 5, 28.0),
        ] {
            sized_motion(&mut p, at(t, n), 10_000, big_frames);
            p.tick(
                at(t, n),
                Link {
                    queue_ms,
                    blocked,
                    ..active(15)
                },
                Cpu::default(),
            );
            assert_eq!(p.crf(), crf, "tick {n}");
        }
    }

    #[test]
    fn tests_that_a_failed_fps_step_up_doubles_its_dwell() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=11 {
            sized_motion(&mut p, at(t, n), 10_000, 5);
            p.tick(
                at(t, n),
                if n == 1 || n == 5 { jam() } else { active(15) },
                Cpu::default(),
            );
            assert_eq!(p.fps(), if n == 4 || n == 11 { 30.0 } else { 20.0 });
        }
    }

    #[test]
    fn tests_that_a_borderline_link_backs_off_to_one_retry_per_twenty_five_ticks() {
        let t = Instant::now();
        let mut p = picture(t);
        let mut changes = Vec::new();
        for n in 1..=120 {
            sized_motion(&mut p, at(t, n), 10_000, 5);
            let link = if p.fps() == 30.0 { jam() } else { active(20) };
            if !p.tick(at(t, n), link, Cpu::default()).is_empty() && n > 60 {
                changes.push(n);
            }
            assert_eq!((p.crf(), p.motion_scale()), (24.0, Scale::Full));
        }
        assert_eq!(changes, [74, 75, 99, 100]);
    }

    #[test]
    fn tests_that_a_reset_preserves_unmeasured_cpu_work() {
        let t = Instant::now();
        let mut p = picture(t);
        motion(&mut p, t, Scale::Full, 70);
        p.tick(t, active(6), Cpu::default());
        motion(&mut p, t + Duration::from_millis(10), Scale::Full, 70);
        p.window_reset(t + Duration::from_millis(20));
        assert_eq!(
            p.cpu.at, 0,
            "only a complete interval can advance CPU pressure"
        );
        let moves = p.tick(at(t, 1), active(6), Cpu::default());
        assert_eq!(moves.len(), 1);
        assert_eq!((moves[0].source, moves[0].why), (Source::Cpu, Why::Work));
    }

    #[test]
    fn tests_that_half_size_vbv_does_not_ratchet_down() {
        let t = Instant::now();
        let mut p = picture(t);
        let half = to_link_half(&mut p, t);
        costly_half_key(&mut p, half);
        let budget = p.link_kbps();
        assert_eq!(budget, 2550); // 85% of the 3 Mb/s delivery.
        for n in 1..=30 {
            let sent_kbps = (p.link_kbps() as f32 * 0.5).min(3000.0);
            p.tick(
                at(half, n),
                Link {
                    blocked: if n % 10 == 0 { 0.4 } else { 0.0 },
                    sent_kbps,
                    delivered_kbps: sent_kbps,
                    base_queue_ms: Some(150),
                    ..active(15)
                },
                Cpu::default(),
            );
            assert_eq!(p.motion_scale(), Scale::Half);
            assert!(p.link_kbps() >= budget, "tick {n}: {} kbps", p.link_kbps());
        }
    }

    #[test]
    fn tests_that_burst_acks_and_clean_ticks_respect_the_bitrate_ceiling() {
        let t = Instant::now();
        let mut p = picture(t);
        p.tick(
            t,
            Link {
                delivered_kbps: 24_000.0,
                ..jam()
            },
            Cpu::default(),
        );
        assert_eq!(p.link_kbps(), 8000);
        p.tick(at(t, 1), active(15), Cpu::default());
        assert_eq!(p.link_kbps(), 8000);
    }

    #[test]
    fn tests_that_a_second_reset_within_ten_seconds_sets_half() {
        let t = Instant::now();
        for (delay, scale) in [(10, Scale::Half), (11, Scale::Full)] {
            let mut p = picture(t);
            for n in [1, 1 + delay] {
                p.window_reset(at(t, n));
                p.tick(at(t, n), Link::default(), Cpu::default());
            }
            assert_eq!(p.motion_scale(), scale);
        }
    }

    #[test]
    fn tests_that_reentry_within_ten_seconds_of_leaving_half_needs_one_tick() {
        let t = Instant::now();
        let mut p = picture(t);
        let half = to_link_half(&mut p, t);
        p.tick(at(half, 2), active(15), Cpu::default());
        assert_eq!(p.motion_scale(), Scale::Full);
        p.tick(at(half, 3), jam(), Cpu::default());
        assert_eq!(p.motion_scale(), Scale::Half);
    }

    #[test]
    fn tests_that_cpu_work_jumps_to_the_fitting_rung() {
        let t = Instant::now();
        for (work_ms, fps) in [(35, 20.0), (42, 15.0)] {
            let mut p = picture(t);
            for n in 1..=2 {
                motion(&mut p, at(t, n), Scale::Full, work_ms);
                p.tick(at(t, n), active(6), Cpu::default());
                assert_eq!(p.fps(), if n == 1 { 30.0 } else { fps });
                assert_eq!((p.crf(), p.motion_scale()), (24.0, Scale::Full));
            }
        }
    }

    #[test]
    fn tests_that_cpu_pressure_never_raises_the_quantizer() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=10 {
            let scale = p.motion_scale();
            motion(&mut p, at(t, n), scale, 90);
            p.tick(at(t, n), active(6), Cpu::default());
            assert_eq!(p.crf(), 24.0);
        }
        assert_eq!((p.fps(), p.motion_scale()), (10.0, Scale::Half));
        for n in 11..=16 {
            let scale = p.motion_scale();
            motion(&mut p, at(t, n), scale, 10);
            p.tick(at(t, n), active(10), Cpu::default());
            assert_eq!(p.fps(), if n < 13 { 10.0 } else { 15.0 });
            assert_eq!(
                p.motion_scale(),
                if n < 16 { Scale::Half } else { Scale::Full }
            );
        }
    }

    #[test]
    fn tests_that_cpu_band_holds_twenty_fps_between_twenty_and_twenty_seven_ms() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=62 {
            let ms = if n <= 2 {
                27
            } else if n % 2 == 0 {
                22
            } else {
                26
            };
            motion(&mut p, at(t, n), Scale::Full, ms);
            p.tick(at(t, n), active(20), Cpu::default());
            assert_eq!(p.fps(), if n == 1 { 30.0 } else { 20.0 });
        }
    }

    #[test]
    fn tests_that_link_steps_from_the_effective_fps() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=2 {
            motion(&mut p, at(t, n), Scale::Full, 48);
            p.tick(at(t, n), active(6), Cpu::default());
        }
        let moves = p.tick(at(t, 3), jam(), Cpu::default());
        assert_eq!(
            (moves[0].from, moves[0].to, p.fps(), p.crf()),
            (0, 3, 15.0, 26.0)
        );
    }

    #[test]
    fn tests_that_throttle_with_kept_cadence_never_lowers_fps() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=60 {
            motion(&mut p, at(t, n), Scale::Full, 10);
            p.tick(
                at(t, n),
                active(30),
                Cpu {
                    throttled: true,
                    ..Cpu::default()
                },
            );
            assert_eq!((p.fps(), p.motion_scale()), (30.0, Scale::Full));
        }
    }

    #[test]
    fn tests_that_throttle_with_missed_cadence_steps_fps() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=60 {
            motion(&mut p, at(t, n), Scale::Full, 10);
            let moves = p.tick(
                at(t, n),
                active(18),
                Cpu {
                    throttled: true,
                    ..Cpu::default()
                },
            );
            if n == 2 {
                assert_eq!((moves[0].why, p.fps()), (Why::Throttled, 20.0));
            }
            assert_eq!(p.motion_scale(), Scale::Full);
            assert!(p.fps() >= 20.0);
        }
    }

    #[test]
    fn tests_that_link_waits_do_not_count_as_cpu_shortfall() {
        let t = Instant::now();
        let mut p = picture(t);
        let cpu = || Cpu {
            stolen: 0.25,
            throttled: true,
        };
        for n in 1..=60 {
            let busy = if n % 2 == 0 {
                Duration::from_millis(500)
            } else {
                Duration::from_millis(499)
            };
            // Twelve frames fit a half second of open credit. Shorter samples are neutral.
            let frames = if n % 2 == 0 { 12 } else { 0 };
            p.tick(
                at(t, n),
                Link {
                    busy,
                    ..active(frames)
                },
                cpu(),
            );
            assert_eq!(p.fps(), 30.0);
        }
        for n in 61..=63 {
            let busy = Duration::from_millis(if n == 62 { 100 } else { 500 });
            p.tick(at(t, n), Link { busy, ..active(11) }, cpu());
        }
        assert_eq!(
            p.fps(),
            20.0,
            "a short sample did not erase the two observed misses"
        );
    }

    #[test]
    fn tests_that_throttle_without_cadence_shortfall_never_sets_half() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=60 {
            let now = at(t, n * 2);
            motion(&mut p, now, Scale::Full, 10);
            // 24 frames over two seconds meet exactly 80% of the floor cadence.
            p.tick(
                now,
                Link {
                    busy: Duration::from_secs(2),
                    ..active(24)
                },
                Cpu {
                    throttled: true,
                    ..Cpu::default()
                },
            );
            assert_eq!(p.motion_scale(), Scale::Full);
        }
        assert_eq!(p.fps(), 15.0);
        for n in 61..=63 {
            p.tick(
                at(t, n * 2),
                Link {
                    busy: Duration::from_secs(2),
                    ..active(23)
                },
                Cpu {
                    throttled: true,
                    ..Cpu::default()
                },
            );
        }
        assert_eq!(p.motion_scale(), Scale::Half);
    }

    #[test]
    fn tests_that_full_size_only_cpu_goes_to_ten_fps_at_full_size() {
        let t = Instant::now();
        let mut p = Picture::new(
            Config {
                adaptive_size: false,
                ..cfg()
            },
            WIDTH,
            ROWS,
            t,
        );
        for n in 1..=7 {
            motion(&mut p, at(t, n), Scale::Full, 70);
            p.tick(at(t, n), active(6), Cpu::default());
            assert_eq!((p.motion_scale(), p.crf()), (Scale::Full, 24.0));
        }
        assert_eq!(p.fps(), 10.0);
        for n in 8..=10 {
            motion(&mut p, at(t, n), Scale::Full, 10);
            p.tick(at(t, n), active(10), Cpu::default());
            assert_eq!(p.fps(), if n < 10 { 10.0 } else { 15.0 });
        }
    }

    #[test]
    fn tests_that_cpu_fps_retries_need_twice_the_calm_evidence() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=13 {
            let work = if matches!(n, 1 | 2 | 6 | 7) { 35 } else { 15 };
            motion(&mut p, at(t, n), Scale::Full, work);
            p.tick(at(t, n), active(15), Cpu::default());
            assert_eq!(
                p.fps(),
                if n == 1 || n == 5 || n == 6 || n == 13 {
                    30.0
                } else {
                    20.0
                }
            );
        }
    }

    #[test]
    fn tests_that_idle_ticks_neither_climb_nor_reset_streaks() {
        let t = Instant::now();
        let mut p = picture(t);
        p.tick(at(t, 1), jam(), Cpu::default());
        for n in [2, 3] {
            motion(&mut p, at(t, n), Scale::Full, 70);
            p.tick(at(t, n), active(8), Cpu::default());
        }
        assert_eq!(p.rung_label(), "link 1 cpu 2");
        for n in 4..=40 {
            assert!(
                p.tick(
                    at(t, n),
                    Link::default(),
                    Cpu {
                        throttled: true,
                        ..Cpu::default()
                    }
                )
                .is_empty()
            );
        }
        motion(&mut p, at(t, 41), Scale::Full, 10);
        let moves = p.tick(at(t, 41), active(8), Cpu::default());
        assert_eq!(moves[0].why, Why::Clean);
        p.tick(at(t, 42), Link::default(), Cpu::default());
        p.tick(at(t, 43), active(8), Cpu::default());
        assert_eq!(p.fps(), 15.0);
        p.tick(at(t, 44), active(8), Cpu::default());
        assert_eq!(p.fps(), 20.0);
    }

    #[test]
    fn tests_that_a_new_pacer_uses_its_own_capacity_for_full_keys() {
        let t = Instant::now();
        for age in [0, 11] {
            let mut p = picture(t);
            let half = to_link_half(&mut p, t);
            costly_half_key(&mut p, half);
            capacity(&mut p, half, 3000.0);
            let changed = at(half, age);
            assert!(!p.full_return_fits(changed));
            p.pacer_changed();
            assert_eq!(p.motion_scale(), Scale::Half);
            assert_eq!(p.capacity_kbps(changed), None);
            assert!(p.full_return_fits(changed));
            capacity(&mut p, changed, 3000.0);
            assert!(
                !p.full_return_fits(changed),
                "the new pacer's probe prices the same key"
            );
            let expired = at(changed, 11);
            assert!(
                p.full_return_fits(expired),
                "only this pacer's jams extend its samples"
            );
            capacity(&mut p, expired, 9000.0);
            p.tick(expired, active(15), Cpu::default());
            assert_eq!(p.plan(expired, BIG).scale, Scale::Full);
        }
    }

    #[test]
    fn tests_that_pacer_replacement_forgets_streaks_and_resize_forgets_costs() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=6 {
            p.tick(at(t, n), jam(), Cpu::default());
        }
        let late = || Link {
            queue_ms: 400,
            ..jam()
        };
        p.tick(at(t, 7), late(), Cpu::default());
        settle(&mut p, at(t, 7));
        p.pacer_changed();
        p.resize(WIDTH, ROWS / 2, at(t, 7));
        assert!(p.estimate(18.0).is_none());
        for n in 8..=10 {
            p.tick(
                at(t, n),
                if n == 8 { late() } else { jam() },
                Cpu::default(),
            );
            assert_eq!(
                p.motion_scale(),
                if n == 10 { Scale::Half } else { Scale::Full }
            );
            assert_eq!((p.fps(), p.crf()), (15.0, 30.0));
        }
    }

    #[test]
    fn tests_that_the_settle_frame_fires_at_the_fps_floor() -> Result<(), &'static str> {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=2 {
            p.tick(at(t, n), jam(), Cpu::default());
        }
        let stop = at(t, 2);
        let plan = p.plan(stop, BIG);
        send(&mut p, stop, plan);
        assert_eq!(p.fps(), 15.0);
        assert!(p.still(stop + Duration::from_millis(249)).is_none());
        let due = stop + Duration::from_millis(250);
        let sharp = p.still(due).ok_or("the sharp frame is due after 250 ms")?;
        assert_eq!(
            sharp,
            Plan {
                scale: Scale::Full,
                sharp_qp: Some(18)
            }
        );
        send(&mut p, due, sharp);
        assert!(p.still(at(stop, 1)).is_none());
        Ok(())
    }

    #[test]
    fn tests_that_tick_reports_each_move_with_its_reason() {
        let t = Instant::now();
        let mut p = picture(t);
        motion(&mut p, at(t, 1), Scale::Full, 35);
        p.tick(at(t, 1), active(6), Cpu::default());
        motion(&mut p, at(t, 2), Scale::Full, 35);
        let moves = p.tick(at(t, 2), jam(), Cpu::default());
        assert_eq!(moves.len(), 2);
        assert_eq!((moves[0].source, moves[0].why), (Source::Link, Why::Severe));
        assert_eq!((moves[1].source, moves[1].why), (Source::Cpu, Why::Work));
        assert_eq!(
            moves[0].to_string(),
            "source=link from=0 to=5 fps=15 crf=30 scale=full why=severe blocked=60 queue_ms=250 offered_kbps=9600 delivered_kbps=3000 work_ms=35 stolen=0 frames=8 busy_ms=400 grabs=8"
        );
    }

    #[test]
    fn tests_that_jitter_without_blockage_never_changes_size() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=60 {
            motion(&mut p, at(t, n), Scale::Full, 15);
            p.tick(
                at(t, n),
                Link {
                    blocked: 0.1,
                    queue_ms: 80,
                    ..Link::default()
                },
                Cpu::default(),
            );
            assert_eq!(p.plan(at(t, n), BIG).scale, Scale::Full);
        }
    }
    #[test]
    fn tests_that_a_capacity_probe_restores_full_within_two_clean_seconds() {
        let t = Instant::now();
        let mut p = picture(t);
        motion(&mut p, t, Scale::Full, 10);
        let entered = to_link_half(&mut p, t);
        assert_eq!(p.plan(entered, BIG).scale, Scale::Half);
        capacity(&mut p, entered, 12_000.0);
        p.tick(at(entered, 2), Link::default(), Cpu::default());
        assert_eq!(
            p.plan(at(entered, 2), BIG).scale,
            Scale::Half,
            "3 Mb/s delivery cannot sustain full motion"
        );
        let recovered = at(entered, 3);
        p.tick(
            recovered,
            Link {
                delivered_kbps: 12_000.0,
                ..jam()
            },
            Cpu::default(),
        );
        p.tick(at(recovered, 1), Link::default(), Cpu::default());
        assert_eq!(p.plan(at(recovered, 1), BIG).scale, Scale::Half);
        p.tick(at(recovered, 2), Link::default(), Cpu::default());
        assert_eq!(p.plan(at(recovered, 2), BIG).scale, Scale::Full);
        assert_eq!((p.fps(), p.crf()), (15.0, 30.0));
    }
    #[test]
    fn tests_that_half_clearing_its_own_queue_does_not_prove_full_capacity() {
        let t = Instant::now();
        let mut p = picture(t);
        motion(&mut p, t, Scale::Full, 10);
        let entered = to_link_half(&mut p, t);
        for n in 1..=8 {
            p.tick(at(entered, n), Link::default(), Cpu::default());
            assert_eq!(p.plan(at(entered, n), BIG).scale, Scale::Half);
        }
    }
    #[test]
    fn tests_that_repeated_failed_full_trials_double_backoff() {
        let t = Instant::now();
        let mut p = picture(t);
        let entered = to_link_half(&mut p, t);
        capacity(&mut p, entered, 20_000.0);
        let t = entered;
        let mut entered = 0;
        for stay in [2, 4, 8, 16] {
            for n in entered + 1..entered + stay {
                p.tick(at(t, n), Link::default(), Cpu::default());
                assert_eq!(
                    p.plan(at(t, n), BIG).scale,
                    Scale::Half,
                    "stay {stay}, second {n}"
                );
            }
            let exit = entered + stay;
            p.tick(at(t, exit), Link::default(), Cpu::default());
            assert_eq!(p.plan(at(t, exit), BIG).scale, Scale::Full);
            entered = exit + 1;
            p.tick(at(t, entered), jam(), Cpu::default());
            assert_eq!(p.plan(at(t, entered), BIG).scale, Scale::Half);
        }
    }
    #[test]
    fn tests_that_persistent_jam_never_probes_full_periodically() {
        let t = Instant::now();
        let mut p = picture(t);
        let entered = to_link_half(&mut p, t);
        for n in 1..=90 {
            p.tick(at(entered, n), jam(), Cpu::default());
            assert_eq!(p.plan(at(entered, n), BIG).scale, Scale::Half);
        }
    }
    #[test]
    fn tests_that_starved_work_reduces_fps_then_size() {
        for work_ms in [54, 70] {
            let t = Instant::now();
            let mut p = picture(t);
            for n in 1..=7 {
                let scale = p.motion_scale();
                motion(&mut p, at(t, n), scale, work_ms);
                p.tick(at(t, n), active(6), Cpu::default());
                assert_eq!(
                    p.plan(at(t, n), BIG).scale,
                    if n < 5 { Scale::Full } else { Scale::Half }
                );
                assert_eq!(
                    p.fps(),
                    if n < 2 {
                        30.0
                    } else if n < 7 {
                        15.0
                    } else {
                        10.0
                    }
                );
                assert_eq!(p.crf(), 24.0);
            }
        }
    }
    #[test]
    fn tests_that_four_work_samples_and_keyframes_do_not_trigger_cpu_pressure() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=5 {
            for _ in 0..4 {
                p.encoded(at(t, n), frame(Scale::Full, 100));
            }
            let mut key = frame(Scale::Full, 200);
            key.key = true;
            p.encoded(at(t, n), key);
            p.tick(at(t, n), Link::default(), Cpu::default());
            assert_eq!(p.plan(at(t, n), BIG).scale, Scale::Full);
        }
    }
    #[test]
    fn tests_that_twenty_five_percent_steal_steps_fps_then_sets_half() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=7 {
            p.tick(
                at(t, n),
                active(6),
                Cpu {
                    stolen: 0.25,
                    throttled: false,
                },
            );
            assert_eq!(
                p.fps(),
                if n < 2 {
                    30.0
                } else if n < 4 {
                    20.0
                } else {
                    15.0
                }
            );
            assert_eq!(
                p.plan(at(t, n), BIG).scale,
                if n < 7 { Scale::Full } else { Scale::Half }
            );
        }
    }
    #[test]
    fn tests_that_healthy_half_interval_work_never_triggers_cpu_guard() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=60 {
            motion(&mut p, at(t, n), Scale::Full, 16);
            p.tick(at(t, n), Link::default(), Cpu::default());
            assert_eq!(p.plan(at(t, n), BIG).scale, Scale::Full);
            assert_eq!(p.fps(), 30.0);
        }
    }
    #[test]
    fn tests_that_cpu_recovery_requires_three_calm_ticks_and_full_cost_headroom() {
        let t = Instant::now();
        let mut p = picture(t);
        let half = to_cpu_half(&mut p, t);
        for n in 1..=3 {
            motion(&mut p, at(half, n), Scale::Half, 12); // Full prediction 42 ms.
            p.tick(at(half, n), active(15), Cpu::default());
            assert_eq!(p.motion_scale(), Scale::Half);
        }
        motion(&mut p, at(half, 4), Scale::Half, 11); // Full prediction 38 ms.
        p.tick(at(half, 4), active(15), Cpu::default());
        assert_eq!((p.motion_scale(), p.fps()), (Scale::Full, 15.0));
    }
    #[test]
    fn tests_that_no_cpu_guard_pins_the_cpu_level_and_preserves_link_adaptation() {
        let t = Instant::now();
        let mut c = cfg();
        c.cpu_guard = false;
        let mut p = Picture::new(c, WIDTH, ROWS, t);
        for n in 1..=4 {
            p.tick(
                at(t, n),
                active(6),
                Cpu {
                    stolen: 0.9,
                    throttled: true,
                },
            );
        }
        assert_eq!(p.plan(at(t, 4), BIG).scale, Scale::Full);
        assert_eq!(p.fps(), 30.0);
        let entered = to_link_half(&mut p, at(t, 4));
        assert_eq!(p.plan(entered, BIG).scale, Scale::Half);
        assert_eq!(p.cpu.at, 0);
    }
    #[test]
    fn tests_that_full_size_only_still_reduces_fps_and_quality() {
        let t = Instant::now();
        let mut c = cfg();
        c.adaptive_size = false;
        let mut p = Picture::new(c, WIDTH, ROWS, t);
        for n in 1..=8 {
            motion(&mut p, at(t, n), Scale::Full, 70);
            p.tick(
                at(t, n),
                jam(),
                Cpu {
                    stolen: 0.9,
                    throttled: true,
                },
            );
        }
        assert_eq!(p.plan(at(t, 8), BIG).scale, Scale::Full);
        assert_eq!((p.fps(), p.crf()), (10.0, 30.0));
    }
    #[test]
    fn tests_that_half_motion_returns_full_after_one_second() {
        let t = Instant::now();
        let mut p = picture(t);
        let entered = to_link_half(&mut p, t);
        let plan = p.plan(entered, BIG);
        send(&mut p, entered, plan);
        assert!(p.still(entered + Duration::from_millis(999)).is_none());
        assert_eq!(
            p.still(at(entered, 1)).map(|plan| plan.scale),
            Some(Scale::Full)
        );
    }

    #[test]
    fn tests_that_drags_every_seven_hundred_ms_keep_cpu_half_for_twenty_seconds() {
        let t = Instant::now();
        let mut p = picture(t);
        settle(&mut p, t);
        let start = to_cpu_half(&mut p, t);
        capacity(&mut p, start, 3000.0);
        p.input(start);
        let half = p.plan(start, BIG);
        assert_eq!(half.scale, Scale::Half);
        send(&mut p, start, half);
        let mut switches = 0;
        for ms in 1..=20_000 {
            let now = start + Duration::from_millis(ms);
            if ms % 1000 == 0 {
                capacity(&mut p, now, 3000.0);
                p.tick(
                    now,
                    Link::default(),
                    Cpu {
                        throttled: true,
                        ..Cpu::default()
                    },
                );
            }
            let plan = if ms % 700 == 0 {
                p.input(now);
                Some(p.plan(now, BIG))
            } else {
                p.still(now)
            };
            if let Some(plan) = plan {
                switches += usize::from(plan.scale != p.scale());
                send(&mut p, now, plan);
            }
        }
        assert_eq!(switches, 0);
        assert_eq!(p.scale_label(), "half(cpu)");
    }
    #[test]
    fn tests_that_caret_blinking_for_thirty_seconds_never_sharpens() {
        let t = Instant::now();
        let mut p = picture(t);
        settle(&mut p, t);
        let mut sharps = 0;
        for n in 1..=60 {
            let now = t + Duration::from_millis(n * 500);
            let plan = p.plan(now, Some((10, 26, 16)));
            sharps += usize::from(plan.sharp_qp.is_some());
            send(&mut p, now, plan);
            if let Some(plan) = p.still(now + Duration::from_millis(250)) {
                sharps += usize::from(plan.sharp_qp.is_some());
                send(&mut p, now, plan);
            }
        }
        assert_eq!(sharps, 0);
    }
    #[test]
    fn tests_that_scroll_then_still_sharpens_exactly_once() {
        let t = Instant::now();
        let mut p = picture(t);
        settle(&mut p, t);
        let plan = p.plan(at(t, 1), BIG);
        send(&mut p, at(t, 1), plan);
        let now = at(t, 1) + Duration::from_millis(250);
        let plan = p.still(now).unwrap();
        assert_eq!(plan.sharp_qp, Some(18));
        send(&mut p, now, plan);
        assert!(p.still(at(t, 2)).is_none());
    }
    #[test]
    fn tests_that_disjoint_soft_rows_accumulate_but_repeated_rows_do_not() {
        let t = Instant::now();
        let mut p = picture(t);
        settle(&mut p, t);
        for (i, start) in [0, 0, 0, 100, 200, 300].into_iter().enumerate() {
            let now = at(t, i as u64 + 1);
            let plan = p.plan(now, Some((start, start + 100, 100)));
            assert!(plan.sharp_qp.is_none());
            send(&mut p, now, plan);
        }
        let plan = p.plan(at(t, 8), Some((400, 500, 100)));
        assert!(plan.sharp_qp.is_none());
        send(&mut p, at(t, 8), plan);
        assert!(
            p.still(at(t, 8) + Duration::from_millis(250))
                .unwrap()
                .sharp_qp
                .is_some()
        );
    }
    #[test]
    fn tests_that_regular_keyframe_owes_a_sharp_frame() {
        let t = Instant::now();
        let mut p = picture(t);
        settle(&mut p, t);
        let mut key = frame(Scale::Full, 10);
        key.key = true;
        p.encoded(at(t, 1), key);
        assert!(
            p.still(at(t, 1) + Duration::from_millis(250))
                .unwrap()
                .sharp_qp
                .is_some()
        );
    }
    #[test]
    fn tests_that_a_regular_keyframe_waits_for_the_requested_sharpen_pause() {
        let t = Instant::now();
        let mut p = picture(t);
        settle(&mut p, t);
        let mut key = frame(Scale::Full, 10);
        key.key = true;
        p.encoded(at(t, 3), key);
        assert!(p.still(at(t, 3)).is_none());
        assert!(p.still(at(t, 3) + Duration::from_millis(249)).is_none());
        assert!(p.still(at(t, 3) + Duration::from_millis(250)).is_some());
    }

    #[test]
    fn tests_that_three_megabit_capacity_budgets_sharp_qp_at_least_twenty_one() {
        let t = Instant::now();
        let mut p = picture(t);
        settle(&mut p, t);
        capacity(&mut p, t, 3000.0);
        let plan = p.plan(at(t, 1), BIG);
        send(&mut p, at(t, 1), plan);
        let q = p
            .still(at(t, 1) + Duration::from_millis(250))
            .unwrap()
            .sharp_qp
            .unwrap();
        assert!(q >= 21, "QP {q}");
    }

    #[test]
    fn tests_that_a_recent_congestion_budgets_the_sharp_frame_conservatively() {
        let t = Instant::now();
        let mut p = picture(t);
        let mut key = frame(Scale::Full, 10);
        key.key = true;
        key.bytes = 25_000;
        p.encoded(t, key);

        let jammed = at(t, 1);
        p.tick(
            jammed,
            Link {
                delivered_kbps: 1000.0,
                ..jam()
            },
            Cpu::default(),
        );
        let now = at(jammed, 3);
        capacity(&mut p, now, 8000.0);
        assert_eq!(
            p.still(now).and_then(|plan| plan.sharp_qp),
            Some(21),
            "at 1 Mb/s, QP 20 needs 317 ms and QP 21 needs 283 ms"
        );
    }

    #[test]
    fn tests_that_a_still_screen_sharpens_after_congestion_expires() {
        let t = Instant::now();
        let mut p = picture(t);
        p.encoded(
            t,
            Encoded {
                key: true,
                ..frame(Scale::Full, 10)
            },
        );
        let jammed = at(t, 1);
        p.tick(
            jammed,
            Link {
                delivered_kbps: 1000.0,
                ..jam()
            },
            Cpu::default(),
        );
        let expiry = at(jammed, 10);
        let retry = p
            .deadline(jammed)
            .expect("a still screen must wake when congestion expires");
        assert!(retry >= expiry && retry <= expiry + Duration::from_millis(1));

        for n in 1..=9 {
            let now = at(jammed, n);
            p.tick(now, Link::default(), Cpu::default());
            assert!(p.still(now).is_none());
            assert_eq!(p.deadline(now), Some(retry));
        }
        assert!(p.still(expiry - Duration::from_nanos(1)).is_none());
        let sharp = p
            .still(retry)
            .expect("the settle frame fits the default 8 Mb/s budget");
        assert_eq!(sharp.sharp_qp, Some(18));
        send(&mut p, retry, sharp);
        assert_eq!(p.deadline(retry), None);
    }

    #[test]
    fn tests_that_a_newer_slow_probe_delays_the_sharp_retry() {
        let t = Instant::now();
        for (bytes, expected_qp) in [(100_000, Some(18)), (400_000, None)] {
            let mut p = picture(t);
            p.encoded(
                t,
                Encoded {
                    key: true,
                    bytes,
                    ..frame(Scale::Full, 10)
                },
            );
            p.tick(t, jam(), Cpu::default());
            capacity(&mut p, at(t, 5), 500.0);

            let after_jam = at(t, 11);
            assert!(p.still(after_jam).is_none());
            let retry = p.deadline(after_jam).expect("the newer probe also expires");
            assert!(retry >= at(t, 15));
            let plan = p.still(retry);
            assert_eq!(plan.and_then(|plan| plan.sharp_qp), expected_qp);
            if let Some(plan) = plan {
                send(&mut p, retry, plan);
            }
            assert_eq!(p.deadline(retry), None);
        }
    }

    #[test]
    fn tests_that_a_clean_link_keeps_the_fast_sharp_budget() {
        let t = Instant::now();
        let mut p = picture(t);
        let mut key = frame(Scale::Full, 10);
        key.key = true;
        p.encoded(t, key);
        capacity(&mut p, t, 8000.0);
        let now = at(t, 1);
        capacity(&mut p, now, 1000.0);
        assert_eq!(
            p.still(now).and_then(|plan| plan.sharp_qp),
            Some(18),
            "the 8 Mb/s peak still prices a clean link after a slower probe"
        );
    }

    #[test]
    fn tests_that_slow_link_sharp_waits_a_second_after_input() {
        let t = Instant::now();
        let mut p = picture(t);
        settle(&mut p, t);
        capacity(&mut p, t, 3000.0);
        let plan = p.plan(at(t, 1), BIG);
        send(&mut p, at(t, 1), plan);
        let input = at(t, 1) + Duration::from_millis(150);
        p.input(input);
        assert!(p.still(at(t, 1) + Duration::from_millis(250)).is_none());
        assert!(p.still(input + Duration::from_millis(999)).is_none());
        assert!(
            p.still(input + Duration::from_secs(1))
                .unwrap()
                .sharp_qp
                .is_some()
        );
    }
    #[test]
    fn tests_that_fast_link_sharp_does_not_wait_for_input_quiet() {
        let t = Instant::now();
        let mut p = picture(t);
        settle(&mut p, t);
        capacity(&mut p, t, 40_000.0);
        let plan = p.plan(at(t, 1), BIG);
        send(&mut p, at(t, 1), plan);
        p.input(at(t, 1) + Duration::from_millis(150));
        assert_eq!(
            p.still(at(t, 1) + Duration::from_millis(250))
                .unwrap()
                .sharp_qp,
            Some(18)
        );
    }
    #[test]
    fn tests_that_sharp_is_skipped_when_budget_cannot_improve_quality() {
        let t = Instant::now();
        let mut p = picture(t);
        settle(&mut p, t);
        capacity(&mut p, t, 100.0);
        let plan = p.plan(at(t, 1), BIG);
        send(&mut p, at(t, 1), plan);
        assert!(p.still(at(t, 2)).is_none());
    }
    #[test]
    fn tests_that_a_moving_thirty_percent_banner_sharpens_once_after_a_scroll() {
        let t = Instant::now();
        let mut p = picture(t);
        settle(&mut p, t);
        let plan = p.plan(at(t, 1), BIG);
        send(&mut p, at(t, 1), plan);
        let mut sharps = 0;
        for n in 1..=90 {
            let now = at(t, 1) + Duration::from_millis(n * 33);
            let rows = ROWS * 3 / 10;
            let plan = p.plan(now, Some((0, rows, rows)));
            sharps += usize::from(plan.sharp_qp.is_some());
            send(&mut p, now, plan);
            assert!(
                p.still(now + Duration::from_millis(16)).is_none(),
                "a gap between animation frames is not a pause"
            );
        }
        assert_eq!(sharps, 1);
        assert!(
            p.still(at(t, 5)).unwrap().sharp_qp.is_some(),
            "once the banner stops, its soft rows can refine"
        );
    }

    #[test]
    fn tests_that_a_caret_and_distant_spinner_do_not_cause_sharp_idrs() {
        let t = Instant::now();
        let mut p = picture(t);
        settle(&mut p, t);
        for n in 1..=60 {
            let now = t + Duration::from_millis(n * 500);
            let plan = p.plan(now, Some((300, 1516, 32)));
            assert!(plan.sharp_qp.is_none());
            send(&mut p, now, plan);
            assert!(p.still(now + Duration::from_millis(250)).is_none());
        }
    }

    #[test]
    fn tests_that_an_expired_capacity_estimate_still_yields_large_sharps_to_input() {
        let t = Instant::now();
        let mut p = picture(t);
        let mut key = frame(Scale::Full, 10);
        key.key = true;
        key.plan.sharp_qp = Some(18);
        key.bytes = 300_000;
        p.encoded(t, key);
        capacity(&mut p, t, 3000.0);
        let now = at(t, 11);
        assert!(p.capacity_kbps(now).is_none());
        let plan = p.plan(now, BIG);
        send(&mut p, now, plan);
        let input = now + Duration::from_millis(150);
        p.input(input);
        assert!(p.still(now + Duration::from_millis(250)).is_none());
        assert!(p.still(input + Duration::from_millis(999)).is_none());
        assert!(
            p.still(input + Duration::from_secs(1))
                .unwrap()
                .sharp_qp
                .is_some()
        );
    }
    fn cpu_second(p: &mut Picture, now: Instant, scale: Scale, work: Work) {
        for _ in 0..6 {
            let mut f = frame(scale, 10);
            f.work = work;
            p.encoded(now, f);
        }
        p.tick(now, active(6), Cpu::default());
    }

    #[test]
    fn tests_that_cpu_half_recovers_with_realistic_conversion_cost_within_five_seconds() {
        let t = Instant::now();
        let mut p = picture(t);
        let half = to_cpu_half(&mut p, t);
        for n in 1..=3 {
            cpu_second(
                &mut p,
                at(half, n),
                Scale::Half,
                Work {
                    grab: Duration::from_millis(2),
                    convert: Duration::from_millis(6),
                    encode: Duration::from_millis(2),
                },
            );
            assert_eq!(
                p.motion_scale(),
                if n < 3 { Scale::Half } else { Scale::Full }
            );
        }
    }

    #[test]
    fn tests_that_cpu_half_trials_full_after_thirty_calm_seconds() {
        for full_ms in [18, 70] {
            let t = Instant::now();
            let mut p = picture(t);
            let t = to_cpu_half(&mut p, t);
            for n in 1..=30 {
                cpu_second(
                    &mut p,
                    at(t, n),
                    Scale::Half,
                    Work {
                        grab: Duration::from_millis(2),
                        convert: Duration::from_millis(6),
                        encode: Duration::from_millis(12),
                    },
                );
                if n < 30 {
                    assert_eq!(p.plan(at(t, n), BIG).scale, Scale::Half);
                }
            }
            assert_eq!(p.plan(at(t, 30), BIG).scale, Scale::Full);
            // No motion means there is no full-cost observation yet.
            p.tick(at(t, 31), Link::default(), Cpu::default());
            assert_eq!(p.plan(at(t, 31), BIG).scale, Scale::Full);
            cpu_second(
                &mut p,
                at(t, 32),
                Scale::Full,
                frame(Scale::Full, full_ms).work,
            );
            assert_eq!(
                p.plan(at(t, 32), BIG).scale,
                if full_ms == 18 {
                    Scale::Full
                } else {
                    Scale::Half
                }
            );
        }
    }

    #[test]
    fn tests_that_spectator_catchup_and_caret_blinks_do_not_sharpen_the_owner() {
        let t = Instant::now();
        let mut p = picture(t);
        settle(&mut p, t);
        let mut sharps = 0;
        for n in 1..=60 {
            let now = t + Duration::from_millis(n * 500);
            if n % 2 == 0 {
                let mut key = frame(Scale::Full, 10);
                key.key = true;
                key.owner_key = false;
                key.big = false;
                p.encoded(now, key);
            }
            let plan = p.plan(now, Some((10, 26, 16)));
            sharps += usize::from(plan.sharp_qp.is_some());
            send(&mut p, now, plan);
            if let Some(plan) = p.still(now + Duration::from_millis(250)) {
                sharps += usize::from(plan.sharp_qp.is_some());
                send(&mut p, now + Duration::from_millis(250), plan);
            }
        }
        assert_eq!(sharps, 0);
    }

    #[test]
    fn tests_that_typing_delays_sharpness_but_not_full_size_recovery() {
        let t = Instant::now();
        let mut p = picture(t);
        settle(&mut p, t);
        let entered = to_link_half(&mut p, t);
        let scroll = entered;
        let plan = p.plan(scroll, BIG);
        assert_eq!(plan.scale, Scale::Half);
        send(&mut p, scroll, plan);
        let first_input = scroll + Duration::from_millis(100);
        let full_at = scroll + Duration::from_secs(1);
        for ms in [100, 600] {
            let now = scroll + Duration::from_millis(ms);
            p.input(now);
            let typed = p.plan(now, Some((10, 26, 16)));
            assert_eq!(typed.scale, Scale::Half);
            send(&mut p, now, typed);
            assert_eq!(p.deadline(now), Some(full_at));
        }
        let full = p
            .still(full_at)
            .expect("full size must not wait for input quiet");
        assert_eq!(
            full,
            Plan {
                scale: Scale::Full,
                sharp_qp: None
            }
        );
        send(&mut p, full_at, full);
        for n in 2..=10 {
            let now = first_input + Duration::from_millis(n * 500);
            p.input(now);
            let plan = p.plan(now, Some((10, 26, 16)));
            assert_eq!(
                plan,
                Plan {
                    scale: Scale::Full,
                    sharp_qp: None
                }
            );
            send(&mut p, now, plan);
            assert!(p.still(now + Duration::from_millis(250)).is_none());
        }
        assert!(
            p.still(first_input + Duration::from_secs(6))
                .unwrap()
                .sharp_qp
                .is_some()
        );
    }
    #[test]
    fn tests_that_a_window_reset_jumps_to_the_quality_cap_without_half() {
        let t = Instant::now();
        let mut p = picture(t);
        let change = p.window_reset(at(t, 1));
        assert_eq!(change.map(|m| m.why), Some(Why::Reset));
        assert!(p.tick(at(t, 1), Link::default(), Cpu::default()).is_empty());
        assert_eq!(
            (p.fps(), p.crf(), p.motion_scale()),
            (15.0, 30.0, Scale::Full)
        );
        p.tick(at(t, 2), jam(), Cpu::default());
        assert_eq!(p.motion_scale(), Scale::Full);
        p.tick(at(t, 3), jam(), Cpu::default());
        assert_eq!(
            p.motion_scale(),
            Scale::Half,
            "the reset counted as the first jam tick"
        );
    }

    #[test]
    fn tests_that_normal_cell_work_with_isolated_peaks_stays_full_for_sixty_seconds() {
        for max_fps in [15.0, 30.0, 60.0] {
            let t = Instant::now();
            let mut c = cfg();
            c.max_fps = max_fps;
            let mut p = Picture::new(c, WIDTH, ROWS, t);
            for n in 1..=60 {
                // Tick p75 varies 30–56 ms; 56 ms crosses 53.33 ms for one tick only.
                let work_ms = [30, 53, 53, 56, 44, 50][(n as usize - 1) % 6];
                motion(&mut p, at(t, n), Scale::Full, work_ms);
                p.tick(at(t, n), Link::default(), Cpu::default());
                assert_eq!(
                    p.plan(at(t, n), BIG).scale,
                    Scale::Full,
                    "max_fps={max_fps}, tick={n}, work={work_ms}ms"
                );
                assert_eq!(p.crf(), 24.0);
                if n >= 2 {
                    assert_eq!(p.fps(), 15.0);
                }
            }
        }
    }

    #[test]
    fn tests_that_cpu_recovery_uses_forty_ms_of_full_work_at_any_fps_cap() {
        for max_fps in [15.0, 30.0, 60.0] {
            for predicted_ms in [40, 41] {
                let t = Instant::now();
                let mut c = cfg();
                c.max_fps = max_fps;
                let mut p = Picture::new(c, WIDTH, ROWS, t);
                let t = to_cpu_half(&mut p, t);
                for n in 1..=3 {
                    cpu_second(
                        &mut p,
                        at(t, n),
                        Scale::Half,
                        Work {
                            grab: Duration::from_millis(predicted_ms - 36),
                            convert: Duration::ZERO,
                            encode: Duration::from_millis(9),
                        },
                    );
                    let expected = if n == 3 && predicted_ms == 40 {
                        Scale::Full
                    } else {
                        Scale::Half
                    };
                    assert_eq!(
                        p.plan(at(t, n), BIG).scale,
                        expected,
                        "max_fps={max_fps}, tick={n}, predicted full={predicted_ms}ms"
                    );
                }
            }
        }
    }

    #[test]
    fn tests_that_throttling_triggers_cpu_half_without_work_samples() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=7 {
            p.tick(
                at(t, n),
                active(10),
                Cpu {
                    throttled: true,
                    ..Cpu::default()
                },
            );
            assert_eq!(
                p.plan(at(t, n), BIG).scale,
                if n < 7 { Scale::Full } else { Scale::Half }
            );
        }
    }

    fn cpu_switches(seconds: u64, normal: bool) -> Vec<u64> {
        let t = Instant::now();
        let mut p = picture(t);
        let mut scale = Scale::Full;
        let mut switches = Vec::new();
        for n in 1..=seconds {
            let work = if scale == Scale::Full {
                frame(
                    scale,
                    if normal {
                        [30, 53, 53, 56, 44, 50][(n as usize - 1) % 6]
                    } else {
                        60
                    },
                )
                .work
            } else {
                // Actual half work11ms; predicted full2+4*9=38ms underestimates60ms.
                Work {
                    grab: Duration::from_millis(2),
                    encode: Duration::from_millis(9),
                    ..Work::default()
                }
            };
            cpu_second(&mut p, at(t, n), scale, work);
            let next = p.plan(at(t, n), BIG).scale;
            if next != scale {
                switches.push(n);
            }
            scale = next;
        }
        switches
    }

    #[test]
    fn tests_that_cpu_size_oscillation_backs_off_from_the_first_scroll() {
        let switches = cpu_switches(240, false);
        eprintln!("CPU oscillation switches at seconds: {switches:?}");
        assert!(
            switches.iter().filter(|&&s| s <= 60).count() <= 3,
            "{switches:?}"
        );
        for start in 60..=120 {
            let count = switches
                .iter()
                .filter(|&&s| s > start && s <= start + 120)
                .count();
            assert!(count <= 4, "window{start}..{}: {switches:?}", start + 120);
        }
    }

    #[test]
    fn tests_that_normal_cells_never_switch_during_the_cpu_backoff_replay() {
        assert!(cpu_switches(240, true).is_empty());
    }

    fn failed_cpu_return(p: &mut Picture, t: Instant) -> Instant {
        let half = to_cpu_half(p, t);
        for n in 1..=3 {
            cpu_second(p, at(half, n), Scale::Half, frame(Scale::Half, 11).work);
        }
        assert_eq!(p.motion_scale(), Scale::Full);
        for n in 4..=6 {
            cpu_second(p, at(half, n), Scale::Full, frame(Scale::Full, 60).work);
        }
        assert_eq!(p.motion_scale(), Scale::Half);
        at(half, 6)
    }

    #[test]
    fn tests_that_measured_full_cost_decays_toward_the_model_with_thirty_second_half_life() {
        let t = Instant::now();
        let mut p = picture(t);
        let t = failed_cpu_return(&mut p, t);
        for n in 1..=30 {
            // Full model20ms:60ms measured floor reaches40ms after30s.
            cpu_second(
                &mut p,
                at(t, n),
                Scale::Half,
                Work {
                    grab: Duration::from_millis(4),
                    encode: Duration::from_millis(4),
                    ..Work::default()
                },
            );
            assert_eq!(
                p.plan(at(t, n), BIG).scale,
                if n < 30 { Scale::Half } else { Scale::Full },
                "tick{n}"
            );
        }
    }

    #[test]
    fn tests_that_quick_cpu_reentries_double_calm_ticks_up_to_sixty() {
        let t = Instant::now();
        let mut p = picture(t);
        let mut n = 0;
        for needed in [3, 6, 12, 24, 48, 60, 60] {
            for _ in 0..if needed == 3 { 7 } else { 3 } {
                n += 1;
                motion(&mut p, at(t, n), Scale::Full, 30);
                p.tick(
                    at(t, n),
                    active(6),
                    Cpu {
                        stolen: 0.25,
                        throttled: false,
                    },
                );
            }
            assert_eq!(p.plan(at(t, n), BIG).scale, Scale::Half);
            for calm in 1..=needed {
                n += 1;
                cpu_second(&mut p, at(t, n), Scale::Half, frame(Scale::Half, 11).work);
                assert_eq!(
                    p.plan(at(t, n), BIG).scale,
                    if calm < needed {
                        Scale::Half
                    } else {
                        Scale::Full
                    },
                    "calm {calm} of {needed}"
                );
            }
        }
    }

    #[test]
    fn tests_that_failed_cpu_trials_wait_sixty_seconds_even_without_a_model_fit() {
        let t = Instant::now();
        let mut p = picture(t);
        let mut n = 0;
        for (wait, starved_ticks) in [(30, 5), (60, 1), (60, 1)] {
            for _ in 0..starved_ticks {
                n += 1;
                cpu_second(&mut p, at(t, n), Scale::Full, frame(Scale::Full, 70).work);
            }
            assert_eq!(p.plan(at(t, n), BIG).scale, Scale::Half);
            for calm in 1..=wait {
                n += 1;
                // Half work 20ms is calm, but predicts 74ms at full size.
                cpu_second(&mut p, at(t, n), Scale::Half, frame(Scale::Half, 20).work);
                assert_eq!(
                    p.plan(at(t, n), BIG).scale,
                    if calm < wait {
                        Scale::Half
                    } else {
                        Scale::Full
                    },
                    "calm {calm} of {wait}"
                );
            }
        }
    }

    #[test]
    fn tests_that_cpu_backoff_resets_only_after_sixty_full_seconds_without_starvation() {
        for clean_seconds in [59, 60] {
            let t = Instant::now();
            let mut p = picture(t);
            let t = failed_cpu_return(&mut p, t);
            // A much cheaper model crosses the measured-cost floor after 30s.
            for n in 1..=30 {
                cpu_second(
                    &mut p,
                    at(t, n),
                    Scale::Half,
                    Work {
                        grab: Duration::from_millis(4),
                        encode: Duration::from_millis(4),
                        ..Work::default()
                    },
                );
            }
            assert_eq!(p.plan(at(t, 30), BIG).scale, Scale::Full);
            for n in 31..=43 {
                cpu_second(&mut p, at(t, n), Scale::Full, frame(Scale::Full, 35).work);
            }
            // One isolated stolen-CPU tick breaks the healthy-full interval.
            motion(&mut p, at(t, 44), Scale::Full, 35);
            p.tick(
                at(t, 44),
                active(6),
                Cpu {
                    stolen: 0.25,
                    throttled: false,
                },
            );
            for n in 45..=45 + clean_seconds {
                cpu_second(&mut p, at(t, n), Scale::Full, frame(Scale::Full, 35).work);
                assert_eq!(p.plan(at(t, n), BIG).scale, Scale::Full);
            }
            let last_full = 45 + clean_seconds;
            for n in last_full + 1..=last_full + 3 {
                cpu_second(&mut p, at(t, n), Scale::Full, frame(Scale::Full, 60).work);
            }
            assert_eq!(p.plan(at(t, last_full + 3), BIG).scale, Scale::Half);
            let needed = if clean_seconds == 60 { 3 } else { 6 };
            for calm in 1..=needed {
                let n = last_full + 3 + calm;
                cpu_second(&mut p, at(t, n), Scale::Half, frame(Scale::Half, 8).work);
                assert_eq!(
                    p.plan(at(t, n), BIG).scale,
                    if calm < needed {
                        Scale::Half
                    } else {
                        Scale::Full
                    },
                    "clean {clean_seconds}, calm {calm}"
                );
            }
        }
    }

    #[test]
    fn tests_that_keyframe_outliers_do_not_delay_full_motion_recovery() {
        let t = Instant::now();
        let mut p = picture(t);
        for _ in 0..5 {
            p.encoded(t, frame(Scale::Full, 10));
        }
        p.tick(at(t, 1), Link::default(), Cpu::default());
        for n in 0..5 {
            let mut f = frame(Scale::Full, 10);
            if n == 0 {
                f.key = true;
                f.bytes *= 5;
            }
            p.encoded(at(t, 1), f);
        }
        let entered = to_link_half(&mut p, at(t, 1));
        let half = p.plan(entered, BIG);
        assert_eq!(half.scale, Scale::Half);
        send(&mut p, entered, half);
        // 12 Mb/s supports 40 KB motion at 30 fps with the required 20% headroom.
        // A single 200 KB key must not inflate that demand or keep motion at half.
        let recovered = at(entered, 1);
        p.tick(
            recovered,
            Link {
                delivered_kbps: 12_000.0,
                ..jam()
            },
            Cpu::default(),
        );
        capacity(&mut p, recovered, 12_000.0);
        p.tick(at(recovered, 1), Link::default(), Cpu::default());
        p.tick(at(recovered, 2), Link::default(), Cpu::default());
        assert_eq!(p.plan(at(recovered, 2), BIG).scale, Scale::Full);
    }

    #[test]
    fn tests_that_cpu_half_motion_every_thirteen_hundred_ms_does_not_switch_size() {
        let t = Instant::now();
        let mut p = picture(t);
        let start = to_cpu_half(&mut p, t);
        let half = p.plan(start, BIG);
        assert_eq!(half.scale, Scale::Half);
        send(&mut p, start, half);
        assert_eq!(p.scale_label(), "half(cpu)");
        let mut switches = 0;
        for ms in 1..=30_000 {
            let now = start + Duration::from_millis(ms);
            if ms % 1000 == 0 {
                p.tick(
                    now,
                    Link::default(),
                    Cpu {
                        throttled: true,
                        ..Cpu::default()
                    },
                );
            }
            let plan = if ms % 1300 == 0 {
                p.input(now);
                Some(p.plan(now, BIG))
            } else {
                p.still(now)
            };
            if let Some(plan) = plan {
                switches += usize::from(p.scale() != plan.scale);
                send(&mut p, now, plan);
            }
        }
        assert_eq!(switches, 0);
        let last_big = start + Duration::from_millis(29_900);
        assert!(p.still(last_big + Duration::from_millis(1999)).is_none());
        let full = p.still(last_big + Duration::from_millis(2100)).unwrap();
        assert_eq!(full.scale, Scale::Full);
        send(&mut p, last_big + Duration::from_millis(2100), full);
        assert_eq!(p.scale(), Scale::Full);
    }
}
