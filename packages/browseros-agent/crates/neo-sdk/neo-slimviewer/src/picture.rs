//! Picture decisions driven by observations and caller-supplied time.
use std::collections::VecDeque;
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
    pub reset: bool,
    pub blocked: f32,
    pub sent_kbps: f32,
    pub delivered_kbps: f32,
    pub queue_ms: u32,
}

#[derive(Default)]
pub struct Cpu {
    pub stolen: f32,
    pub throttled: bool,
}

const FULL_SIZE_AFTER: Duration = Duration::from_secs(1);
const CPU_FULL_SIZE_AFTER: Duration = Duration::from_secs(2);
const CAPACITY_WINDOW: Duration = Duration::from_secs(10);
// 80% and 60% of a 15 fps budget, independent of the configured cap.
// Integer nanoseconds keep the 40 ms recovery boundary inclusive.
const CPU_STARVED_WORK: Duration = Duration::from_nanos(800_000_000 / 15);
const CPU_FULL_WORK: Duration = Duration::from_nanos(600_000_000 / 15);

impl Scale {
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
    width: usize,
    soft: Vec<bool>,
    soft_minimum: usize,
    scale: Scale,
    half_cpu: bool,
    crf: f32,
    fps: f32,
    link_kbps: u32,
    last_big: Instant,
    last_change: Instant,
    newest_damage: Option<Instant>,
    planned_change: Option<Instant>,
    calm_sharp: bool,
    last_input: Option<Instant>,
    link_half: Option<Instant>,
    last_congested: Option<Instant>,
    last_full: Option<Instant>,
    backoff: Duration,
    capacity: VecDeque<(Instant, f32)>,
    last_delivery: Option<(Instant, f64)>,
    demand_kbps: Option<f32>,
    full_big: usize,
    full_big_bytes: u64,
    cpu_half: bool,
    cpu_trial: bool,
    cpu_calm_needed: u8,
    cpu_return_pending: bool,
    cpu_returned_at: Option<Instant>,
    cpu_full_calm_since: Option<Instant>,
    cpu_full_floor: Option<(Instant, Duration)>,
    last_starved: Instant,
    sampled_full: usize,
    starved_ticks: u8,
    calm_ticks: u8,
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
            width,
            soft: vec![false; height],
            soft_minimum: 0,
            scale: Scale::Full,
            half_cpu: false,
            crf: cfg.base_crf,
            fps: cfg.max_fps,
            link_kbps: 2000,
            last_big: now,
            last_change: now,
            newest_damage: None,
            planned_change: None,
            calm_sharp: false,
            last_input: None,
            link_half: None,
            last_congested: None,
            last_full: None,
            backoff: Duration::from_secs(2),
            capacity: VecDeque::new(),
            last_delivery: None,
            demand_kbps: None,
            full_big: 0,
            full_big_bytes: 0,
            cpu_half: false,
            cpu_trial: false,
            cpu_calm_needed: 3,
            cpu_return_pending: false,
            cpu_returned_at: None,
            cpu_full_calm_since: None,
            cpu_full_floor: None,
            last_starved: now,
            sampled_full: 0,
            starved_ticks: 0,
            calm_ticks: 0,
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
        self.crf
    }
    pub fn fps(&self) -> f32 {
        self.fps
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
            Scale::Half if self.half_cpu => "half(cpu)",
            Scale::Half => "half(link)",
        }
    }

    pub fn input(&mut self, now: Instant) {
        self.last_input = Some(now);
    }

    pub fn damage(&mut self, now: Instant) {
        self.newest_damage = Some(now);
    }

    fn motion_scale(&self) -> Scale {
        if self.cfg.adaptive_size && (self.cpu_half || self.link_half.is_some()) {
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
                < Duration::from_secs_f32(1.0 / self.fps)
        {
            return false;
        }
        let Some((at, seconds_per_byte)) = self.last_delivery else {
            return false;
        };
        if now.saturating_duration_since(at) > CAPACITY_WINDOW {
            return false;
        }
        let due = self.last_big + self.full_size_after();
        now < due
            && self.estimate(self.crf).is_some_and(|bytes| {
                f64::from(bytes) * 0.25 * seconds_per_byte
                    > due.saturating_duration_since(now).as_secs_f64()
            })
    }

    /// Next useful wakeup. A rejected sharp budget has no deadline until new link facts arrive.
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
        if self.scale == Scale::Half && self.half_cpu {
            CPU_FULL_SIZE_AFTER
        } else {
            FULL_SIZE_AFTER
        }
    }

    fn deadline_since(&self, now: Instant, since: Instant) -> Option<Instant> {
        if self.scale == Scale::Half {
            return Some(self.last_big + self.full_size_after());
        }
        self.sharp_deadline_since(now, since)
    }

    fn sharp_deadline_since(&self, now: Instant, since: Instant) -> Option<Instant> {
        let pause = if self.scale == Scale::Half {
            self.full_size_after()
        } else {
            self.cfg.sharpen_after
        };
        let due = since + pause;
        if self.needs_sharp()
            && let Some((_, transfer)) = self.sharp_budget(now)
        {
            let quiet = if transfer > Duration::from_millis(150) {
                self.last_input.map(|at| at + Duration::from_secs(1))
            } else {
                None
            };
            return Some(quiet.map_or(due, |quiet| due.max(quiet)));
        }
        None
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

    fn sharp_budget(&self, now: Instant) -> Option<(u8, Duration)> {
        let cap = self.capacity_kbps(now).unwrap_or(8000.0);
        for qp in self.cfg.sharpen_qp..=51 {
            if f32::from(qp) >= self.crf - 2.0 {
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
            self.half_cpu = self.cpu_half;
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
                qp: frame.plan.sharp_qp.map_or(self.crf, f32::from),
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

    pub fn probe(&mut self, now: Instant, bytes: usize, delay: Duration, rtt: Duration) {
        if bytes >= 32 * 1024 && !delay.is_zero() {
            // Include RTT: this budgets the whole wait for the next key's ACK.
            self.last_delivery = Some((now, delay.as_secs_f64() / bytes as f64));
        }
        if bytes >= 32 * 1024
            && let Some(transfer) = delay.checked_sub(rtt)
            && !transfer.is_zero()
        {
            self.capacity
                .push_back((now, bytes as f32 * 8.0 / 1000.0 / transfer.as_secs_f32()));
        }
    }

    pub fn capacity_kbps(&self, now: Instant) -> Option<f32> {
        self.capacity
            .iter()
            .filter(|(at, _)| now.saturating_duration_since(*at) <= CAPACITY_WINDOW)
            .map(|(_, kbps)| *kbps)
            .reduce(f32::max)
    }

    pub fn tick(&mut self, now: Instant, link: Link, cpu: Cpu) {
        while self
            .capacity
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > CAPACITY_WINDOW)
        {
            self.capacity.pop_front();
        }
        if self.full_big >= 5 {
            let demand =
                self.full_big_bytes as f32 / self.full_big as f32 * 8.0 * self.fps / 1000.0;
            self.demand_kbps = Some(
                self.demand_kbps
                    .map_or(demand, |old| 0.75 * old + 0.25 * demand),
            );
        }
        self.full_big = 0;
        self.full_big_bytes = 0;
        let congested = link.reset || link_full(link.blocked);
        if congested {
            self.last_congested = Some(now);
            if link.delivered_kbps > 0.0 {
                self.capacity.push_back((now, link.delivered_kbps));
            }
            // The previous tick must have spent quality headroom before size changes.
            if self.cfg.adaptive_size
                && self.crf >= self.cfg.base_crf + 8.0
                && self.link_half.is_none()
            {
                if self
                    .last_full
                    .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(10))
                {
                    self.backoff = (self.backoff * 2).min(Duration::from_secs(16));
                }
                self.link_half = Some(now);
            }
            let offered = self.demand_kbps.unwrap_or(link.sent_kbps);
            let step = if offered > 0.0 && link.delivered_kbps > 0.0 {
                (6.0 * (offered / (0.85 * link.delivered_kbps)).log2())
                    .round()
                    .clamp(1.0, 8.0)
            } else if offered > 0.0 {
                // A congested tick with known demand and zero delivery is a stalled link.
                8.0
            } else {
                3.0
            };
            self.crf += step;
        } else {
            if link.queue_ms > 150 {
                self.crf += 2.0;
            } else if link.queue_ms < 40 {
                self.crf -= 1.0;
            }
            let clean = self
                .last_congested
                .map_or(Duration::MAX, |at| now.saturating_duration_since(at));
            let capacity_ok = self
                .capacity_kbps(now)
                .zip(self.demand_kbps)
                .is_none_or(|(cap, demand)| cap >= demand * 1.2);
            if self
                .link_half
                .is_some_and(|at| now.saturating_duration_since(at) >= self.backoff)
                && clean >= Duration::from_secs(2)
                && capacity_ok
            {
                self.link_half = None;
                self.last_full = Some(now);
            }
            if clean >= Duration::from_secs(30) {
                self.backoff = Duration::from_secs(2);
            }
        }
        self.crf = self
            .crf
            .clamp(self.cfg.base_crf, self.cfg.base_crf + self.cfg.crf_range);
        if congested && link.delivered_kbps > 100.0 {
            self.link_kbps = (link.delivered_kbps * 0.85) as u32;
        } else if !congested {
            self.link_kbps = (self.link_kbps + self.link_kbps / 10).min(8000);
        }
        self.link_kbps = self.link_kbps.max(300);

        let work = percentile75(&mut self.work);
        self.work_p75_ms = work.unwrap_or_default().as_secs_f32() * 1000.0;
        if self.full_work.len() >= 5 {
            self.predicted_full = percentile75(&mut self.full_work);
        }
        let starved = (self.work.len() >= 5 && work.is_some_and(|w| w > CPU_STARVED_WORK))
            || cpu.stolen > 0.20
            || cpu.throttled;
        let full_observed = self.sampled_full >= 5 && self.sampled_full == self.work.len();
        self.sampled_full = 0;
        self.work.clear();
        self.full_work.clear();
        if !self.cfg.cpu_guard {
            return;
        }
        let recent_return = self
            .cpu_returned_at
            .is_some_and(|at| now.saturating_duration_since(at) <= Duration::from_secs(10));
        if starved {
            self.cpu_full_calm_since = None;
            if recent_return && full_observed {
                self.cpu_full_floor = work.map(|cost| (now, cost));
            }
            self.last_starved = now;
            self.calm_ticks = 0;
            self.starved_ticks += 1;
            if self.starved_ticks >= 2 || (self.cpu_trial && full_observed) {
                if self.cfg.adaptive_size && !self.cpu_half {
                    if recent_return {
                        self.cpu_calm_needed = (self.cpu_calm_needed * 2).min(60);
                    }
                    self.cpu_half = true;
                    self.cpu_trial = false;
                    self.cpu_return_pending = false;
                } else {
                    self.fps = (self.fps * 0.8).max(10.0).min(self.cfg.max_fps);
                }
                self.starved_ticks = 0;
            }
        } else {
            if full_observed {
                self.cpu_trial = false;
            }
            self.starved_ticks = 0;
            self.calm_ticks = self.calm_ticks.saturating_add(1);
            self.fps = (self.fps + 2.0).min(self.cfg.max_fps);
            if self.cpu_half && self.calm_ticks >= self.cpu_calm_needed {
                let trial_wait = Duration::from_secs(u64::from(self.cpu_calm_needed) * 10)
                    .min(Duration::from_secs(60));
                if self
                    .cpu_full_prediction(now)
                    .is_some_and(|w| w <= CPU_FULL_WORK)
                {
                    self.cpu_half = false;
                } else if now.saturating_duration_since(self.last_starved) >= trial_wait {
                    self.cpu_half = false;
                    self.cpu_trial = true;
                }
                if !self.cpu_half {
                    self.cpu_return_pending = true;
                    self.cpu_returned_at = None;
                }
            }
            if self.scale == Scale::Full && !self.cpu_half && !self.cpu_return_pending {
                let since = *self.cpu_full_calm_since.get_or_insert(now);
                if now.saturating_duration_since(since) >= Duration::from_secs(60) {
                    self.cpu_calm_needed = 3;
                    self.cpu_full_floor = None;
                }
            } else {
                self.cpu_full_calm_since = None;
            }
        }
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
            crf_range: 12.0,
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
            reset: false,
            queue_ms: 250,
        }
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
    fn tests_that_threefold_jam_spends_quality_before_half_on_second_tick() {
        let t = Instant::now();
        let mut p = picture(t);
        motion(&mut p, t, Scale::Full, 10);
        p.tick(at(t, 1), jam(), Cpu::default());
        assert!(p.crf() >= 32.0);
        assert_eq!(p.plan(at(t, 1), BIG).scale, Scale::Full);
        p.tick(at(t, 2), jam(), Cpu::default());
        assert_eq!(p.plan(at(t, 2), BIG).scale, Scale::Half);
    }
    #[test]
    fn tests_that_missing_rate_data_raises_crf_by_three() {
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
        assert_eq!(p.crf(), 27.0);
    }
    #[test]
    fn tests_that_queue_rules_and_crf_range_remain_in_force() {
        let t = Instant::now();
        let mut p = picture(t);
        p.tick(
            at(t, 1),
            Link {
                reset: false,
                queue_ms: 151,
                ..Link::default()
            },
            Cpu::default(),
        );
        assert_eq!(p.crf(), 26.0);
        p.tick(at(t, 2), Link::default(), Cpu::default());
        assert_eq!(p.crf(), 25.0);
        for n in 3..20 {
            p.tick(at(t, n), jam(), Cpu::default());
        }
        assert_eq!(p.crf(), 36.0);
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
                    reset: false,
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
        p.tick(at(t, 1), jam(), Cpu::default());
        p.tick(at(t, 2), jam(), Cpu::default());
        assert_eq!(p.plan(at(t, 2), BIG).scale, Scale::Half);
        capacity(&mut p, at(t, 2), 12_000.0);
        p.tick(at(t, 3), Link::default(), Cpu::default());
        assert_eq!(p.plan(at(t, 3), BIG).scale, Scale::Half);
        p.tick(at(t, 4), Link::default(), Cpu::default());
        assert_eq!(p.plan(at(t, 4), BIG).scale, Scale::Full);
    }
    #[test]
    fn tests_that_half_clearing_its_own_queue_does_not_prove_full_capacity() {
        let t = Instant::now();
        let mut p = picture(t);
        motion(&mut p, t, Scale::Full, 10);
        p.tick(at(t, 1), jam(), Cpu::default());
        p.tick(at(t, 2), jam(), Cpu::default());
        for n in 3..=10 {
            p.tick(at(t, n), Link::default(), Cpu::default());
            assert_eq!(p.plan(at(t, n), BIG).scale, Scale::Half);
        }
    }
    #[test]
    fn tests_that_repeated_failed_full_trials_double_backoff() {
        let t = Instant::now();
        let mut p = picture(t);
        p.tick(at(t, 1), jam(), Cpu::default());
        p.tick(at(t, 2), jam(), Cpu::default());
        capacity(&mut p, at(t, 2), 20_000.0);
        let mut entered = 2;
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
            if p.plan(at(t, entered), BIG).scale == Scale::Full {
                entered += 1;
                p.tick(at(t, entered), jam(), Cpu::default());
            }
            assert_eq!(p.plan(at(t, entered), BIG).scale, Scale::Half);
        }
    }
    #[test]
    fn tests_that_persistent_jam_never_probes_full_periodically() {
        let t = Instant::now();
        let mut p = picture(t);
        p.tick(at(t, 1), jam(), Cpu::default());
        for n in 2..=90 {
            p.tick(at(t, n), jam(), Cpu::default());
            assert_eq!(p.plan(at(t, n), BIG).scale, Scale::Half);
        }
    }
    #[test]
    fn tests_that_starved_work_reduces_size_then_fps() {
        for work_ms in [54, 70] {
            let t = Instant::now();
            let mut p = picture(t);
            for n in 1..=4 {
                let scale = if n <= 2 { Scale::Full } else { Scale::Half };
                motion(&mut p, at(t, n), scale, work_ms);
                p.tick(at(t, n), Link::default(), Cpu::default());
                assert_eq!(
                    p.plan(at(t, n), BIG).scale,
                    if n < 2 { Scale::Full } else { Scale::Half }
                );
                assert_eq!(p.fps(), if n < 4 { 30.0 } else { 24.0 });
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
    fn tests_that_twenty_five_percent_steal_triggers_half() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=2 {
            p.tick(
                at(t, n),
                Link::default(),
                Cpu {
                    stolen: 0.25,
                    throttled: false,
                },
            );
        }
        assert_eq!(p.plan(at(t, 2), BIG).scale, Scale::Half);
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
        for n in 1..=4 {
            motion(
                &mut p,
                at(t, n),
                if n <= 2 { Scale::Full } else { Scale::Half },
                70,
            );
            p.tick(at(t, n), Link::default(), Cpu::default());
        }
        for n in 5..=7 {
            motion(&mut p, at(t, n), Scale::Half, 12); // full prediction 42 ms: no headroom
            p.tick(at(t, n), Link::default(), Cpu::default());
            assert_eq!(p.plan(at(t, n), BIG).scale, Scale::Half);
        }
        assert_eq!(p.fps(), 30.0);
        motion(&mut p, at(t, 8), Scale::Half, 11); // 2 + 4*9 = 38 ms
        p.tick(at(t, 8), Link::default(), Cpu::default());
        assert_eq!(p.plan(at(t, 8), BIG).scale, Scale::Full);
    }
    #[test]
    fn tests_that_disabled_cpu_guard_preserves_link_adaptation() {
        let t = Instant::now();
        let mut c = cfg();
        c.cpu_guard = false;
        let mut p = Picture::new(c, WIDTH, ROWS, t);
        for n in 1..=4 {
            p.tick(
                at(t, n),
                Link::default(),
                Cpu {
                    stolen: 0.9,
                    throttled: true,
                },
            );
        }
        assert_eq!(p.plan(at(t, 4), BIG).scale, Scale::Full);
        p.tick(at(t, 5), jam(), Cpu::default());
        p.tick(at(t, 6), jam(), Cpu::default());
        assert_eq!(p.plan(at(t, 6), BIG).scale, Scale::Half);
    }
    #[test]
    fn tests_that_full_size_only_still_reduces_fps_and_quality() {
        let t = Instant::now();
        let mut c = cfg();
        c.adaptive_size = false;
        let mut p = Picture::new(c, WIDTH, ROWS, t);
        for n in 1..=8 {
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
        assert!(p.fps() < 30.0);
        assert!(p.fps() >= 10.0);
        assert_eq!(p.crf(), 36.0);
    }
    #[test]
    fn tests_that_half_motion_returns_full_after_one_second() {
        let t = Instant::now();
        let mut p = picture(t);
        p.tick(at(t, 1), jam(), Cpu::default());
        p.tick(at(t, 2), jam(), Cpu::default());
        let plan = p.plan(at(t, 2), BIG);
        send(&mut p, at(t, 2), plan);
        assert!(p.still(at(t, 2) + Duration::from_millis(999)).is_none());
        assert_eq!(p.still(at(t, 3)).unwrap().scale, Scale::Full);
    }

    #[test]
    fn tests_that_drags_every_seven_hundred_ms_keep_cpu_half_for_twenty_seconds() {
        let t = Instant::now();
        let mut p = picture(t);
        settle(&mut p, t);
        for n in 1..=2 {
            motion(&mut p, at(t, n), Scale::Full, 70);
            p.tick(at(t, n), Link::default(), Cpu::default());
        }
        let start = at(t, 2);
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
        p.tick(now, Link::default(), Cpu::default());
    }

    #[test]
    fn tests_that_cpu_half_recovers_with_realistic_conversion_cost_within_five_seconds() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=2 {
            cpu_second(
                &mut p,
                at(t, n),
                Scale::Full,
                Work {
                    grab: Duration::from_millis(2),
                    convert: Duration::from_millis(6),
                    encode: Duration::from_millis(62),
                },
            );
        }
        assert_eq!(p.plan(at(t, 2), BIG).scale, Scale::Half);
        for n in 3..=5 {
            cpu_second(
                &mut p,
                at(t, n),
                Scale::Half,
                Work {
                    grab: Duration::from_millis(2),
                    convert: Duration::from_millis(6),
                    encode: Duration::from_millis(2),
                },
            );
        }
        assert_eq!(p.plan(at(t, 5), BIG).scale, Scale::Full);
    }

    #[test]
    fn tests_that_cpu_half_trials_full_after_thirty_calm_seconds() {
        for full_ms in [18, 70] {
            let t = Instant::now();
            let mut p = picture(t);
            for n in 1..=2 {
                cpu_second(&mut p, at(t, n), Scale::Full, frame(Scale::Full, 70).work);
            }
            for n in 3..=32 {
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
                if n < 32 {
                    assert_eq!(p.plan(at(t, n), BIG).scale, Scale::Half);
                }
            }
            assert_eq!(p.plan(at(t, 32), BIG).scale, Scale::Full);
            // No motion means there is no full-cost observation yet.
            p.tick(at(t, 33), Link::default(), Cpu::default());
            assert_eq!(p.plan(at(t, 33), BIG).scale, Scale::Full);
            cpu_second(
                &mut p,
                at(t, 34),
                Scale::Full,
                frame(Scale::Full, full_ms).work,
            );
            assert_eq!(
                p.plan(at(t, 34), BIG).scale,
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
        p.tick(at(t, 1), jam(), Cpu::default());
        p.tick(at(t, 2), jam(), Cpu::default());
        let scroll = at(t, 2);
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
    fn tests_that_window_resets_spend_quality_even_without_ack_samples() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=4 {
            p.tick(
                at(t, n),
                Link {
                    reset: true,
                    ..Link::default()
                },
                Cpu::default(),
            );
        }
        assert!(p.crf() >= 32.0);
        assert_eq!(p.plan(at(t, 4), BIG).scale, Scale::Half);
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
                assert_eq!(p.fps(), max_fps);
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
                for n in 1..=2 {
                    cpu_second(&mut p, at(t, n), Scale::Full, frame(Scale::Full, 70).work);
                }
                assert_eq!(p.plan(at(t, 2), BIG).scale, Scale::Half);
                for n in 3..=5 {
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
                    let expected = if n == 5 && predicted_ms == 40 {
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
        for n in 1..=2 {
            p.tick(
                at(t, n),
                Link::default(),
                Cpu {
                    throttled: true,
                    ..Cpu::default()
                },
            );
            assert_eq!(
                p.plan(at(t, n), BIG).scale,
                if n == 1 { Scale::Full } else { Scale::Half }
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

    fn failed_cpu_return(p: &mut Picture, t: Instant) {
        for n in 1..=2 {
            cpu_second(p, at(t, n), Scale::Full, frame(Scale::Full, 60).work);
        }
        assert_eq!(p.plan(at(t, 2), BIG).scale, Scale::Half);
        for n in 3..=5 {
            cpu_second(
                p,
                at(t, n),
                Scale::Half,
                Work {
                    grab: Duration::from_millis(2),
                    encode: Duration::from_millis(9),
                    ..Work::default()
                },
            );
        }
        assert_eq!(p.plan(at(t, 5), BIG).scale, Scale::Full);
        for n in 6..=7 {
            cpu_second(p, at(t, n), Scale::Full, frame(Scale::Full, 60).work);
        }
        assert_eq!(p.plan(at(t, 7), BIG).scale, Scale::Half);
    }

    #[test]
    fn tests_that_measured_full_cost_decays_toward_the_model_with_thirty_second_half_life() {
        let t = Instant::now();
        let mut p = picture(t);
        failed_cpu_return(&mut p, t);
        for n in 8..=37 {
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
                if n < 37 { Scale::Half } else { Scale::Full },
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
            for _ in 0..2 {
                n += 1;
                motion(&mut p, at(t, n), Scale::Full, 30);
                p.tick(
                    at(t, n),
                    Link::default(),
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
        for (wait, starved_ticks) in [(30, 2), (60, 1), (60, 1)] {
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
            failed_cpu_return(&mut p, t);
            // A much cheaper model crosses the measured-cost floor after 30s.
            for n in 8..=37 {
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
            assert_eq!(p.plan(at(t, 37), BIG).scale, Scale::Full);
            for n in 38..=50 {
                cpu_second(&mut p, at(t, n), Scale::Full, frame(Scale::Full, 30).work);
            }
            // One isolated stolen-CPU tick breaks the healthy-full interval.
            motion(&mut p, at(t, 51), Scale::Full, 30);
            p.tick(
                at(t, 51),
                Link::default(),
                Cpu {
                    stolen: 0.25,
                    throttled: false,
                },
            );
            for n in 52..=52 + clean_seconds {
                cpu_second(&mut p, at(t, n), Scale::Full, frame(Scale::Full, 30).work);
                assert_eq!(p.plan(at(t, n), BIG).scale, Scale::Full);
            }
            let last_full = 52 + clean_seconds;
            for n in last_full + 1..=last_full + 2 {
                cpu_second(&mut p, at(t, n), Scale::Full, frame(Scale::Full, 60).work);
            }
            assert_eq!(p.plan(at(t, last_full + 2), BIG).scale, Scale::Half);
            let needed = if clean_seconds == 60 { 3 } else { 6 };
            for calm in 1..=needed {
                let n = last_full + 2 + calm;
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
        p.tick(at(t, 2), jam(), Cpu::default());
        p.tick(at(t, 3), jam(), Cpu::default());
        let half = p.plan(at(t, 3), BIG);
        assert_eq!(half.scale, Scale::Half);
        send(&mut p, at(t, 3), half);
        // 12 Mb/s supports 40 KB motion at 30 fps with the required 20% headroom.
        // A single 200 KB key must not inflate that demand or keep motion at half.
        capacity(&mut p, at(t, 3), 12_000.0);
        p.tick(at(t, 4), Link::default(), Cpu::default());
        p.tick(at(t, 5), Link::default(), Cpu::default());
        assert_eq!(p.plan(at(t, 5), BIG).scale, Scale::Full);
    }

    #[test]
    fn tests_that_cpu_half_motion_every_thirteen_hundred_ms_does_not_switch_size() {
        let t = Instant::now();
        let mut p = picture(t);
        for n in 1..=2 {
            motion(&mut p, at(t, n), Scale::Full, 70);
            p.tick(at(t, n), Link::default(), Cpu::default());
        }
        let start = at(t, 2);
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
