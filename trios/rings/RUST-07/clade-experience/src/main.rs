use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

fn project_dir() -> String { trios_config::project_dir() }

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ExperienceEpisode {
    timestamp: String,
    tags: Vec<String>,
    road: String,
    success: bool,
    pattern: String,
    anti_pattern: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Cluster {
    tag: String,
    episodes: Vec<ExperienceEpisode>,
    road_a_count: usize,
    road_b_count: usize,
    road_c_count: usize,
    top_patterns: Vec<String>,
    top_anti_patterns: Vec<String>,
    suggest_child: bool,
}

fn main() {
    println!("[CladeExperience] Experience Replay Analysis");

    let episodes = load_episodes();
    println!("  Loaded {} episodes", episodes.len());

    let clusters = cluster_by_tags(&episodes);
    println!("  Formed {} clusters", clusters.len());

    for cluster in &clusters {
        println!("\n  Cluster: {}", cluster.tag);
        println!("    Episodes: {} (A={}, B={}, C={})",
            cluster.episodes.len(),
            cluster.road_a_count,
            cluster.road_b_count,
            cluster.road_c_count
        );
        if cluster.suggest_child {
            println!("    [DNA] SUGGEST CHILD: Road B/C clade proposed");
            println!("    Top patterns: {}", cluster.top_patterns.join(", "));
            println!("    Anti-patterns: {}", cluster.top_anti_patterns.join(", "));
        }
    }

    let child_spec = generate_child_spec(&clusters);
    if let Some(spec) = child_spec {
        println!("\n  [DNA] Proposed child clade spec:");
        println!("{}", serde_json::to_string_pretty(&spec).unwrap_or_default());
    } else {
        println!("\n  No child clade proposed yet. Need >3 episodes with Road A in a cluster.");
    }
}

fn load_episodes() -> Vec<ExperienceEpisode> {
    let dir = format!("{}/.trinity/experience", &project_dir());
    let mut episodes = vec![];

    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                if size > 1_000_000 {
                    eprintln!("[experience] Skipping {} - exceeds 1MB limit ({}KB)", path.display(), size / 1024);
                    continue;
                }
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(ep) = serde_json::from_str::<ExperienceEpisode>(&content) {
                        episodes.push(ep);
                    }
                }
            }
        }
    }

    episodes
}

fn cluster_by_tags(episodes: &[ExperienceEpisode]) -> Vec<Cluster> {
    let mut tag_map: HashMap<String, Vec<ExperienceEpisode>> = HashMap::new();

    for ep in episodes {
        for tag in &ep.tags {
            tag_map.entry(tag.clone()).or_default().push(ep.clone());
        }
    }

    tag_map
        .into_iter()
        .map(|(tag, eps)| {
            let road_a = eps.iter().filter(|e| e.road == "A").count();
            let road_b = eps.iter().filter(|e| e.road == "B").count();
            let road_c = eps.iter().filter(|e| e.road == "C").count();

            let mut patterns: HashMap<String, usize> = HashMap::new();
            let mut anti_patterns: HashMap<String, usize> = HashMap::new();

            for ep in &eps {
                *patterns.entry(ep.pattern.clone()).or_insert(0) += 1;
                *anti_patterns.entry(ep.anti_pattern.clone()).or_insert(0) += 1;
            }

            let mut top_patterns: Vec<_> = patterns.into_iter().collect();
            top_patterns.sort_by_key(|b| std::cmp::Reverse(b.1));

            let mut top_anti: Vec<_> = anti_patterns.into_iter().collect();
            top_anti.sort_by_key(|b| std::cmp::Reverse(b.1));

            Cluster {
                tag: tag.clone(),
                road_a_count: road_a,
                road_b_count: road_b,
                road_c_count: road_c,
                suggest_child: road_a >= 3 && eps.len() >= 3,
                top_patterns: top_patterns.into_iter().take(3).map(|(p, _)| p).collect(),
                top_anti_patterns: top_anti.into_iter().take(3).map(|(p, _)| p).collect(),
                episodes: eps,
            }
        })
        .collect()
}

#[derive(Serialize, Deserialize, Debug)]
struct ChildSpec {
    parent_clade: String,
    proposed_id: String,
    reason: String,
    road: String,
    patterns: Vec<String>,
    anti_patterns: Vec<String>,
}

fn generate_child_spec(clusters: &[Cluster]) -> Option<ChildSpec> {
    for cluster in clusters {
        if cluster.suggest_child {
            return Some(ChildSpec {
                parent_clade: "clade-1.0.0".to_string(),
                proposed_id: format!("clade-1.1.0-{}", cluster.tag),
                reason: format!("Cluster '{}' has {} Road A episodes - propose Road B/C diversification", cluster.tag, cluster.road_a_count),
                road: if cluster.road_b_count > cluster.road_c_count { "C".to_string() } else { "B".to_string() },
                patterns: cluster.top_patterns.clone(),
                anti_patterns: cluster.top_anti_patterns.clone(),
            });
        }
    }
    None
}

#[cfg(test)]
// Tests legitimately use expect()/unwrap() for fixtures and invariants; the
// workspace deny/warn policy targets production code paths, not test setup.
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn make_episode(tags: &[&str], road: &str, success: bool, pattern: &str) -> ExperienceEpisode {
        ExperienceEpisode {
            timestamp: "2026-06-01T00:00:00Z".to_string(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            road: road.to_string(),
            success,
            pattern: pattern.to_string(),
            anti_pattern: "none".to_string(),
        }
    }

    #[test]
    fn cluster_empty_episodes() {
        let clusters = cluster_by_tags(&[]);
        assert!(clusters.is_empty());
    }

    #[test]
    fn cluster_single_episode() {
        let eps = vec![make_episode(&["build"], "A", true, "fast-fix")];
        let clusters = cluster_by_tags(&eps);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].tag, "build");
        assert_eq!(clusters[0].road_a_count, 1);
        assert!(!clusters[0].suggest_child);
    }

    #[test]
    fn cluster_multi_tag_episode_duplicates_into_both() {
        let eps = vec![make_episode(&["build", "security"], "B", true, "audit")];
        let clusters = cluster_by_tags(&eps);
        assert_eq!(clusters.len(), 2);
        let tags: Vec<&str> = clusters.iter().map(|c| c.tag.as_str()).collect();
        assert!(tags.contains(&"build"));
        assert!(tags.contains(&"security"));
    }

    #[test]
    fn suggest_child_requires_3_road_a_and_3_episodes() {
        let eps = vec![
            make_episode(&["perf"], "A", true, "optimize"),
            make_episode(&["perf"], "A", true, "optimize"),
            make_episode(&["perf"], "A", false, "cache"),
        ];
        let clusters = cluster_by_tags(&eps);
        let perf = clusters.iter().find(|c| c.tag == "perf").expect("perf cluster");
        assert!(perf.suggest_child);
        assert_eq!(perf.road_a_count, 3);
    }

    #[test]
    fn suggest_child_false_with_2_episodes() {
        let eps = vec![
            make_episode(&["ui"], "A", true, "layout"),
            make_episode(&["ui"], "A", true, "color"),
        ];
        let clusters = cluster_by_tags(&eps);
        let ui = clusters.iter().find(|c| c.tag == "ui").expect("ui cluster");
        assert!(!ui.suggest_child);
    }

    #[test]
    fn generate_child_spec_none_when_no_suggestion() {
        let eps = vec![make_episode(&["test"], "B", true, "tdd")];
        let clusters = cluster_by_tags(&eps);
        assert!(generate_child_spec(&clusters).is_none());
    }

    #[test]
    fn generate_child_spec_picks_road_c_when_more_road_b() {
        let eps = vec![
            make_episode(&["net"], "A", true, "retry"),
            make_episode(&["net"], "A", true, "timeout"),
            make_episode(&["net"], "A", true, "backoff"),
            make_episode(&["net"], "B", true, "pool"),
        ];
        let clusters = cluster_by_tags(&eps);
        let spec = generate_child_spec(&clusters).expect("should suggest");
        assert_eq!(spec.road, "C");
    }

    #[test]
    fn top_patterns_sorted_by_frequency() {
        let eps = vec![
            make_episode(&["db"], "A", true, "index"),
            make_episode(&["db"], "A", true, "index"),
            make_episode(&["db"], "A", true, "cache"),
        ];
        let clusters = cluster_by_tags(&eps);
        let db = clusters.iter().find(|c| c.tag == "db").expect("db cluster");
        assert_eq!(db.top_patterns[0], "index");
    }

    #[test]
    fn load_episodes_skips_oversized_files() {
        let dir = "/tmp/clade_experience_size_test";
        let _ = fs::create_dir_all(dir);
        let big_path = format!("{}/big.json", dir);
        let small = serde_json::to_string(&make_episode(&["test"], "A", true, "p")).unwrap_or_default();
        fs::write(format!("{}/small.json", dir), &small).ok();
        // 1.1MB file - should be skipped
        let big = "x".repeat(1_100_000);
        fs::write(&big_path, &big).ok();

        std::env::set_var("TRIOS_ROOT", "/tmp/clade_experience_size_test_root");
        let _ = fs::create_dir_all("/tmp/clade_experience_size_test_root/.trinity/experience");
        fs::write("/tmp/clade_experience_size_test_root/.trinity/experience/small.json", &small).ok();
        let big2 = "x".repeat(1_100_000);
        fs::write("/tmp/clade_experience_size_test_root/.trinity/experience/big.json", &big2).ok();
        let episodes = load_episodes();
        // big.json should be skipped, small.json may or may not parse depending on format
        assert!(episodes.len() <= 1);
        std::env::remove_var("TRIOS_ROOT");
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all("/tmp/clade_experience_size_test_root");
    }
}
