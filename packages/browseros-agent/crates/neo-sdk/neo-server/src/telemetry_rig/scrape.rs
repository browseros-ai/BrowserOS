//! A Prometheus text exposition (0.0.4), parsed just far enough to read one series' value: what
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
    value: f64,
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

    /// `neo_tabs_open` by origin, as `(agent, found, neo)`; a series not served reads as 0.
    pub(crate) fn open_tabs(&self) -> (f64, f64, f64) {
        let open = |origin| self.count("neo_tabs_open", &[("origin", origin)]);
        (open("agent"), open("found"), open("neo"))
    }

    /// The value of the series named `name` whose labels are exactly `labels`, if it is there.
    pub(crate) fn get(&self, name: &str, labels: &[(&str, &str)]) -> Option<f64> {
        let wanted: BTreeMap<String, String> = labels
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        self.samples
            .iter()
            .find(|sample| sample.name == name && sample.labels == wanted)
            .map(|sample| sample.value)
    }

    /// As [`Scrape::get`], with an absent series read as 0: a counter nobody bumped yet.
    pub(crate) fn count(&self, name: &str, labels: &[(&str, &str)]) -> f64 {
        self.get(name, labels).unwrap_or(0.0)
    }

    /// Every non-zero series of the families in `names`, one `name{key="value",..} value` line
    /// each, labels sorted, lines sorted: a whole-set comparison shows any series too many as
    /// plainly as one missing.
    pub(crate) fn lines_of(&self, names: &[&str]) -> Vec<String> {
        let mut lines: Vec<String> = self
            .samples
            .iter()
            .filter(|sample| names.contains(&sample.name.as_str()) && sample.value != 0.0)
            .map(|sample| {
                let labels: Vec<String> = sample
                    .labels
                    .iter()
                    .map(|(key, value)| format!("{key}=\"{value}\""))
                    .collect();
                if labels.is_empty() {
                    format!("{} {}", sample.name, sample.value)
                } else {
                    format!("{}{{{}}} {}", sample.name, labels.join(","), sample.value)
                }
            })
            .collect();
        lines.sort();
        lines
    }

    /// Every label set series `name` has, each as `key=value` pairs, for a failure message or a
    /// check that no unexpected series appeared.
    pub(crate) fn label_sets(&self, name: &str) -> Vec<Vec<(String, String)>> {
        self.samples
            .iter()
            .filter(|sample| sample.name == name)
            .map(|sample| sample.labels.clone().into_iter().collect())
            .collect()
    }
}

/// `name{key="value",...} 12.5` or `name 3`. Neo's label values never hold a quote or a comma,
/// so the split below is exact for its own output.
fn sample(line: &str) -> Option<Sample> {
    let (series, value) = line.rsplit_once(' ')?;
    let value = match value {
        "+Inf" => f64::INFINITY,
        other => other.parse().ok()?,
    };
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
        value,
    })
}
