//! Image subsets of large datasets: a deterministic share by name hash, plus the BarBeR
//! development "trap" list (docs/datasets.md#barber-development-subset).

use std::collections::BTreeSet;

use serde::Deserialize;

/// Versioned trap images, embedded so the bench works from any directory.
pub const DEV_EXTRA_JSON: &str = include_str!("../barber-dev-extra.json");

/// Share of BarBeR kept by hash in the development subset (1/N).
pub const DEV_SHARE: u64 = 4;

#[derive(Debug, Deserialize)]
struct DevExtra {
    images: Vec<DevImage>,
}

#[derive(Debug, Deserialize)]
struct DevImage {
    file: String,
    #[allow(dead_code)]
    reason: String,
}

/// File names listed in a `barber-dev-extra.json` document.
pub fn parse_dev_extra(json: &str) -> Result<BTreeSet<String>, String> {
    let doc: DevExtra =
        serde_json::from_str(json).map_err(|e| format!("barber-dev-extra.json: {e}"))?;
    Ok(doc.images.into_iter().map(|i| i.file).collect())
}

/// Keeps an image when an FNV-1a hash of its name falls in the 1/`n` share; `n` of 0 or 1 keeps all.
pub fn in_subset(file: &str, n: u64) -> bool {
    n <= 1 || {
        let h = file.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
        (h >> 16) % n == 0
    }
}

/// Which images of a dataset run: a hash share of 1/`share`, plus every name in `extra`.
#[derive(Debug, Clone)]
pub struct Subset {
    share: u64,
    extra: BTreeSet<String>,
}

impl Subset {
    /// Every image.
    pub fn all() -> Self {
        Self::share(1)
    }

    /// The 1/`n` hash share only.
    pub fn share(n: u64) -> Self {
        Self {
            share: n,
            extra: BTreeSet::new(),
        }
    }

    /// The BarBeR development subset: the 1/4 hash share united with the trap list.
    pub fn dev() -> Result<Self, String> {
        Ok(Self {
            share: DEV_SHARE,
            extra: parse_dev_extra(DEV_EXTRA_JSON)?,
        })
    }

    pub fn contains(&self, file: &str) -> bool {
        in_subset(file, self.share) || self.extra.contains(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_trap_list_parses_and_has_no_duplicates() {
        let doc: DevExtra = serde_json::from_str(DEV_EXTRA_JSON).unwrap();
        let names = parse_dev_extra(DEV_EXTRA_JSON).unwrap();
        assert_eq!(names.len(), doc.images.len());
    }

    #[test]
    fn dev_subset_contains_every_trap_image_and_is_deterministic() {
        let dev = Subset::dev().unwrap();
        for file in parse_dev_extra(DEV_EXTRA_JSON).unwrap() {
            assert!(dev.contains(&file), "{file}");
        }
        let again = Subset::dev().unwrap();
        for i in 0..2000 {
            let name = format!("image_{i}.jpg");
            assert_eq!(dev.contains(&name), again.contains(&name));
        }
    }

    #[test]
    fn dev_subset_is_the_union_of_share_and_list() {
        let dev = Subset::dev().unwrap();
        let share = Subset::share(DEV_SHARE);
        let extra = parse_dev_extra(DEV_EXTRA_JSON).unwrap();
        let names: Vec<String> = (0..4000).map(|i| format!("img{i}.png")).collect();
        let kept = names.iter().filter(|n| share.contains(n)).count();
        assert!((600..1400).contains(&kept), "kept {kept} of 4000");
        for n in &names {
            assert_eq!(dev.contains(n), share.contains(n) || extra.contains(n));
        }
    }

    #[test]
    fn extra_names_outside_the_share_are_kept() {
        let mut dev = Subset::share(4);
        let outside = (0..)
            .map(|i| format!("x{i}.jpg"))
            .find(|n| !in_subset(n, 4))
            .unwrap();
        assert!(!dev.contains(&outside));
        dev.extra.insert(outside.clone());
        assert!(dev.contains(&outside));
    }

    #[test]
    fn share_of_one_keeps_everything() {
        assert!(Subset::all().contains("anything.jpg"));
    }
}
