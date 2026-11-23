//! A Prometheus text exposition (0.0.4), parsed just far enough to read series values: what
//! a scraper of `GET /v1/metrics` sees. The format's own rules (`# HELP` / `# TYPE`, histogram
//! buckets) are neo-telemetry's tests' business; this reads numbers back.

use std::collections::BTreeMap;

use neo_telemetry::Telemetry;

/// Every sample of one scrape. A series is its family name plus its exact label set.
#[derive(Debug)]
pub(crate) struct Scrape {
    samples: Vec<Sample>,
}

#[derive(Debug)]
struct Sample {
    name: String,
    labels: BTreeMap<String, String>,
    /// The value as written: `3`, `0.25`, `+Inf`.
    text: String,
}

impl Scrape {
    /// Reads `text`; any sample line it cannot read fails the test, so a malformed exposition
    /// never passes as an empty one. The error is a plain message, so `?` lifts it into either
    /// of the crate's test error types.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let samples = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| sample(line).ok_or_else(|| format!("not a sample line: {line:?}")))
            .collect::<Result<_, _>>()?;
        Ok(Self { samples })
    }

    /// What `telemetry` would serve on `GET /v1/metrics` now.
    pub(crate) fn of(telemetry: &Telemetry) -> Result<Self, String> {
        Self::parse(&telemetry.render_prometheus())
    }

    /// `neo_tab_count` by origin, as `(agent, found, neo)`; a series not served reads as 0.
    pub(crate) fn open_tabs(&self) -> (u64, u64, u64) {
        let open = |origin| {
            self.whole("neo_tab_count", &[("neo_tab_origin", origin)])
                .unwrap_or(0)
        };
        (open("agent"), open("found"), open("neo"))
    }

    /// The value of a series that holds a whole number (a counter, a gauge, a count), compared
    /// exactly; `None` when the series is absent or not whole.
    pub(crate) fn whole(&self, name: &str, labels: &[(&str, &str)]) -> Option<u64> {
        self.find(name, labels)?.text.parse().ok()
    }

    /// The series named `name` whose labels are exactly `labels`, if it is there.
    fn find(&self, name: &str, labels: &[(&str, &str)]) -> Option<&Sample> {
        let wanted: BTreeMap<String, String> = labels
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        self.samples
            .iter()
            .find(|sample| sample.name == name && sample.labels == wanted)
    }

    /// Every non-zero series of the families in `names`, one `name{key="value",..} value` line
    /// each, labels sorted, lines sorted: a whole-set comparison shows any series too many as
    /// plainly as one missing.
    pub(crate) fn lines_of(&self, names: &[&str]) -> Vec<String> {
        let mut lines: Vec<String> = self
            .samples
            .iter()
            .filter(|sample| names.contains(&sample.name.as_str()) && sample.text != "0")
            .map(|sample| {
                let labels: Vec<String> = sample
                    .labels
                    .iter()
                    .map(|(key, value)| format!("{key}=\"{value}\""))
                    .collect();
                if labels.is_empty() {
                    format!("{} {}", sample.name, sample.text)
                } else {
                    format!("{}{{{}}} {}", sample.name, labels.join(","), sample.text)
                }
            })
            .collect();
        lines.sort();
        lines
    }
}

/// `name{key="value",...} 12.5` or `name 3`. Neo's label values never hold a quote or a comma,
/// so the split below is exact for its own output.
fn sample(line: &str) -> Option<Sample> {
    let (series, text) = line.rsplit_once(' ')?;
    if text != "+Inf" && text.parse::<f64>().is_err() {
        return None;
    }
    let (name, labels) = match series.split_once('{') {
        None => (series, BTreeMap::new()),
        Some((name, rest)) => {
            let inner = rest.strip_suffix('}')?;
            let labels = inner
                .split(',')
                .filter(|pair| !pair.is_empty())
                .map(|pair| {
                    let (key, quoted) = pair.split_once('=')?;
                    let value = quoted.strip_prefix('"')?.strip_suffix('"')?;
                    Some((key.to_owned(), value.to_owned()))
                })
                .collect::<Option<_>>()?;
            (name, labels)
        }
    };
    Some(Sample {
        name: name.to_owned(),
        labels,
        text: text.to_owned(),
    })
}
