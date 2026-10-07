//! Regression gate: versioned baselines of read counts and the check against them.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub expected: usize,
    pub read: usize,
    pub false_positives: usize,
}

/// Counts per group (dataset or video) and item (symbology or engine).
pub type Groups = BTreeMap<String, BTreeMap<String, Counts>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    pub version: u32,
    /// Part of the baseline key: a baseline only applies to the effort it was recorded at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    pub groups: Groups,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub failures: Vec<String>,
    pub notices: Vec<String>,
}

impl Outcome {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    pub fn print(&self) {
        for n in &self.notices {
            println!("notice: {n}");
        }
        for f in &self.failures {
            println!("FAIL: {f}");
        }
        if self.passed() {
            println!("gate: PASS");
        } else {
            println!("gate: FAIL ({} problem(s))", self.failures.len());
        }
    }
}

impl Baseline {
    pub fn new(effort: Option<&str>, groups: Groups) -> Self {
        Self {
            version: FORMAT_VERSION,
            effort: effort.map(str::to_owned),
            groups,
        }
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let b: Self =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if b.version != FORMAT_VERSION {
            return Err(format!(
                "{}: unsupported baseline version {}",
                path.display(),
                b.version
            ));
        }
        Ok(b)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        text.push('\n');
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Compares `current` against this baseline. Latency is never part of it.
    pub fn check(&self, effort: Option<&str>, current: &Groups) -> Outcome {
        let mut out = Outcome::default();
        if self.effort.as_deref() != effort {
            out.failures.push(format!(
                "baseline was recorded at effort {}, this run used {}",
                self.effort.as_deref().unwrap_or("(none)"),
                effort.unwrap_or("(none)")
            ));
            return out;
        }
        for (group, items) in &self.groups {
            let Some(cur_items) = current.get(group) else {
                out.failures
                    .push(format!("{group}: present in baseline but not in this run"));
                continue;
            };
            for (item, base) in items {
                let cur = cur_items.get(item).copied().unwrap_or(Counts {
                    expected: 0,
                    read: 0,
                    false_positives: 0,
                });
                let label = format!("{group} / {item}");
                if cur.expected != base.expected {
                    out.failures.push(format!(
                        "{label}: expected count changed {} -> {} (dataset differs from baseline)",
                        base.expected, cur.expected
                    ));
                } else if cur.read < base.read {
                    out.failures.push(format!(
                        "{label}: read {} -> {} of {} (regression of {})",
                        base.read,
                        cur.read,
                        cur.expected,
                        base.read - cur.read
                    ));
                } else if cur.read > base.read {
                    out.notices.push(format!(
                        "{label}: read {} -> {} of {}; consider updating the baseline (--write-baseline)",
                        base.read, cur.read, cur.expected
                    ));
                }
                if cur.false_positives > 0 {
                    out.failures.push(format!(
                        "{label}: {} false positive(s)",
                        cur.false_positives
                    ));
                }
            }
            for (item, cur) in cur_items {
                if !items.contains_key(item) {
                    if cur.false_positives > 0 {
                        out.failures.push(format!(
                            "{group} / {item}: {} false positive(s)",
                            cur.false_positives
                        ));
                    } else {
                        out.notices.push(format!(
                            "{group} / {item}: not in baseline (read {} of {})",
                            cur.read, cur.expected
                        ));
                    }
                }
            }
        }
        for (group, cur_items) in current {
            if !self.groups.contains_key(group) {
                let fp: usize = cur_items.values().map(|c| c.false_positives).sum();
                if fp > 0 {
                    out.failures.push(format!(
                        "{group}: not in baseline and has {fp} false positive(s)"
                    ));
                } else {
                    out.notices.push(format!("{group}: not in baseline"));
                }
            }
        }
        out
    }

    /// Refuses to record a baseline that already contains false positives.
    pub fn validate_for_write(groups: &Groups) -> Result<(), String> {
        let bad: Vec<String> = groups
            .iter()
            .flat_map(|(g, items)| {
                items
                    .iter()
                    .filter(|(_, c)| c.false_positives > 0)
                    .map(move |(i, c)| format!("{g} / {i}: {} FP", c.false_positives))
            })
            .collect();
        if bad.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "refusing to write a baseline with false positives:\n  {}",
                bad.join("\n  ")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(expected: usize, read: usize, fp: usize) -> Counts {
        Counts {
            expected,
            read,
            false_positives: fp,
        }
    }

    fn groups(entries: &[(&str, &str, Counts)]) -> Groups {
        let mut g = Groups::new();
        for (grp, item, counts) in entries {
            g.entry((*grp).into())
                .or_default()
                .insert((*item).into(), *counts);
        }
        g
    }

    fn base() -> Baseline {
        Baseline::new(
            Some("Fast"),
            groups(&[("a", "qr", c(10, 8, 0)), ("a", "ean13", c(5, 5, 0))]),
        )
    }

    #[test]
    fn identical_passes() {
        let cur = base().groups;
        let o = base().check(Some("Fast"), &cur);
        assert!(o.passed() && o.notices.is_empty());
    }

    #[test]
    fn lower_read_fails() {
        let cur = groups(&[("a", "qr", c(10, 7, 0)), ("a", "ean13", c(5, 5, 0))]);
        let o = base().check(Some("Fast"), &cur);
        assert_eq!(o.failures.len(), 1);
        assert!(o.failures[0].contains("a / qr"));
    }

    #[test]
    fn higher_read_notifies() {
        let cur = groups(&[("a", "qr", c(10, 9, 0)), ("a", "ean13", c(5, 5, 0))]);
        let o = base().check(Some("Fast"), &cur);
        assert!(o.passed());
        assert_eq!(o.notices.len(), 1);
    }

    #[test]
    fn false_positive_fails_even_when_reads_improve() {
        let cur = groups(&[("a", "qr", c(10, 9, 1)), ("a", "ean13", c(5, 5, 0))]);
        let o = base().check(Some("Fast"), &cur);
        assert!(!o.passed());
        let cur = groups(&[
            ("a", "qr", c(10, 8, 0)),
            ("a", "ean13", c(5, 5, 0)),
            ("a", "code39", c(0, 0, 2)),
            ("new", "qr", c(1, 1, 1)),
        ]);
        assert_eq!(base().check(Some("Fast"), &cur).failures.len(), 2);
    }

    #[test]
    fn missing_group_or_item_fails() {
        let o = base().check(Some("Fast"), &Groups::new());
        assert!(!o.passed());
        let cur = groups(&[("a", "qr", c(10, 8, 0))]);
        let o = base().check(Some("Fast"), &cur);
        assert_eq!(o.failures.len(), 1);
        assert!(o.failures[0].contains("ean13"));
    }

    #[test]
    fn changed_expected_and_effort_mismatch_fail() {
        let cur = groups(&[("a", "qr", c(11, 8, 0)), ("a", "ean13", c(5, 5, 0))]);
        assert!(!base().check(Some("Fast"), &cur).passed());
        assert!(!base().check(Some("Balanced"), &base().groups).passed());
        assert!(!base().check(None, &base().groups).passed());
    }

    #[test]
    fn new_clean_entries_are_notices() {
        let mut cur = base().groups;
        cur.insert("b".into(), groups(&[("b", "qr", c(1, 1, 0))])["b"].clone());
        let o = base().check(Some("Fast"), &cur);
        assert!(o.passed());
        assert_eq!(o.notices.len(), 1);
    }

    #[test]
    fn write_validation_and_roundtrip() {
        assert!(Baseline::validate_for_write(&groups(&[("a", "qr", c(1, 1, 1))])).is_err());
        let b = base();
        let text = serde_json::to_string(&b).unwrap();
        assert_eq!(serde_json::from_str::<Baseline>(&text).unwrap(), b);
    }
}
