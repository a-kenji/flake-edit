//! Rendering for `follow --stats` and `--dry-run` summaries.

use std::collections::{BTreeMap, HashMap};

use crate::follows::AttrPath;
use crate::lock::NestedInput;
use crate::lock::sizes::InputSizes;

use super::AppliedPlan;

/// The summary of what this run changed (or would change).
pub(super) fn applied_summary(
    applied: &AppliedPlan,
    wrote: bool,
    stats: bool,
    nested_inputs: &[NestedInput],
    sizes: &InputSizes,
) -> String {
    let mut out = String::new();

    let lock_routes: HashMap<&AttrPath, &AttrPath> = if stats {
        nested_inputs
            .iter()
            .filter_map(|nested| nested.follows.as_ref().map(|target| (&nested.path, target)))
            .collect()
    } else {
        HashMap::new()
    };
    let realized: Vec<&(AttrPath, AttrPath)> = applied
        .applied_follows
        .iter()
        .filter(|(path, target)| lock_routes.get(path) != Some(&target))
        .collect();

    if !realized.is_empty() {
        let verb = if wrote {
            "Deduplicated"
        } else {
            "Would deduplicate"
        };
        let noun = if realized.len() == 1 {
            "input"
        } else {
            "inputs"
        };
        let entry_sizes: Vec<Option<u64>> = realized
            .iter()
            .map(|(path, _)| if stats { sizes.size_for(path) } else { None })
            .collect();
        let suffix = if stats {
            sum_suffix(&entry_sizes, false)
        } else {
            String::new()
        };
        out.push_str(&format!("{verb} {} {noun}{suffix}.\n", realized.len()));
        for ((path, target), size) in realized.iter().zip(&entry_sizes) {
            out.push_str(&format!(
                "  {path} -> {target}{}\n",
                entry_suffix(*size, stats)
            ));
        }
    }

    if !applied.unfollowed.is_empty() {
        let verb = if wrote { "Removed" } else { "Would remove" };
        let noun = if applied.unfollowed.len() == 1 {
            "declaration"
        } else {
            "declarations"
        };
        out.push_str(&format!(
            "{verb} {} stale follows {noun}.\n",
            applied.unfollowed.len()
        ));
        for path in &applied.unfollowed {
            out.push_str(&format!("  {path} (input no longer exists)\n"));
        }
    }

    out
}

/// The summary of follows already existing in `flake.lock`.
/// Grouped by the store path they resolve to.
pub(super) fn existing_summary(
    nested_inputs: &[NestedInput],
    sizes: &InputSizes,
) -> Option<String> {
    let entries: Vec<(&AttrPath, &AttrPath)> = nested_inputs
        .iter()
        .filter_map(|nested| nested.follows.as_ref().map(|target| (&nested.path, target)))
        .collect();
    if entries.is_empty() {
        return None;
    }

    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for (path, target) in &entries {
        let store_path = sizes.store_path_for(path);
        let key = store_path.map_or_else(|| format!("target:{target}"), str::to_string);
        let label = store_path
            .and_then(|sp| sizes.label_for_store_path(sp))
            .map_or_else(|| target.to_string(), str::to_string);
        let group = groups.entry(key).or_insert_with(|| Group {
            label,
            count: 0,
            unit_size: sizes.size_for(path),
        });
        group.count += 1;
    }

    let group_totals: Vec<Option<u64>> = groups.values().map(Group::total).collect();
    let count = entries.len();
    let input_noun = if count == 1 { "input" } else { "inputs" };
    let target_noun = if groups.len() == 1 {
        "target"
    } else {
        "targets"
    };

    let mut out = format!(
        "Already deduplicated by existing follows: {count} {input_noun} across {} {target_noun}{}.\n",
        groups.len(),
        sum_suffix(&group_totals, true),
    );

    // Biggest savings first, then by label.
    let mut rows: Vec<&Group> = groups.values().collect();
    rows.sort_by(|a, b| {
        b.total()
            .cmp(&a.total())
            .then_with(|| a.label.cmp(&b.label))
    });
    for group in rows {
        let unit = match group.unit_size {
            Some(bytes) => format!("~{}", format_size(bytes)),
            None => "size unknown".to_string(),
        };
        if group.count == 1 {
            out.push_str(&format!("  {} ({unit})\n", group.label));
        } else {
            let total = group
                .total()
                .map(|bytes| format!(" (~{})", format_size(bytes)))
                .unwrap_or_default();
            out.push_str(&format!(
                "  {} ({unit}) x {}{total}\n",
                group.label, group.count
            ));
        }
    }

    Some(out)
}

/// One row of [`existing_summary`].
struct Group {
    label: String,
    count: u64,
    unit_size: Option<u64>,
}

impl Group {
    fn total(&self) -> Option<u64> {
        self.unit_size.map(|size| size.saturating_mul(self.count))
    }
}

fn sum_suffix(entry_sizes: &[Option<u64>], hypothetical: bool) -> String {
    let known: Vec<u64> = entry_sizes.iter().flatten().copied().collect();
    if known.is_empty() {
        return " (size unknown)".to_string();
    }
    let total = known
        .iter()
        .fold(0u64, |acc, size| acc.saturating_add(*size));
    let qualifier = if known.len() == entry_sizes.len() {
        ""
    } else {
        "at least "
    };
    let size = format_size(total);
    if hypothetical {
        format!(" ({qualifier}~{size})")
    } else {
        format!(" ({qualifier}{size} saved)")
    }
}

fn entry_suffix(size: Option<u64>, stats: bool) -> String {
    if !stats {
        return String::new();
    }
    match size {
        Some(bytes) => format!(" ({})", format_size(bytes)),
        None => " (size unknown)".to_string(),
    }
}

/// Human-readable sizes.
fn format_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{} MiB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{} KiB", bytes / KIB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ap(s: &str) -> AttrPath {
        s.parse().unwrap()
    }

    fn nested(path: &str, follows: Option<&str>) -> NestedInput {
        NestedInput {
            path: ap(path),
            follows: follows.map(ap),
            url: None,
        }
    }

    fn applied(follows: &[(&str, &str)], unfollowed: &[&str]) -> AppliedPlan {
        AppliedPlan {
            applied_follows: follows.iter().map(|(s, t)| (ap(s), ap(t))).collect(),
            unfollowed: unfollowed.iter().map(|s| ap(s)).collect(),
            ..AppliedPlan::default()
        }
    }

    #[test]
    fn format_size_picks_unit_by_scale() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(2048), "2 KiB");
        assert_eq!(format_size(150 * 1024 * 1024), "150 MiB");
        assert_eq!(
            format_size(1024 * 1024 * 1024 + 512 * 1024 * 1024),
            "1.5 GiB"
        );
    }

    #[test]
    fn sum_suffix_reports_partial_and_unknown_sums() {
        assert_eq!(sum_suffix(&[None, None], false), " (size unknown)");
        assert_eq!(sum_suffix(&[None, None], true), " (size unknown)");
        assert_eq!(
            sum_suffix(&[Some(100 * 1024 * 1024), Some(50 * 1024 * 1024)], false),
            " (150 MiB saved)",
        );
        assert_eq!(
            sum_suffix(&[Some(100 * 1024 * 1024), Some(50 * 1024 * 1024)], true),
            " (~150 MiB)",
        );
        assert_eq!(
            sum_suffix(&[Some(100 * 1024 * 1024), None], false),
            " (at least 100 MiB saved)",
        );
        assert_eq!(
            sum_suffix(&[Some(100 * 1024 * 1024), None], true),
            " (at least ~100 MiB)",
        );
    }

    #[test]
    fn applied_summary_without_stats_renders_plain_summary() {
        let plan = applied(
            &[("home-manager.nixpkgs", "nixpkgs")],
            &["crane.flake-compat"],
        );
        let out = applied_summary(&plan, true, false, &[], &InputSizes::default());
        assert_eq!(
            out,
            "Deduplicated 1 input.\n\
             \x20 home-manager.nixpkgs -> nixpkgs\n\
             Removed 1 stale follows declaration.\n\
             \x20 crane.flake-compat (input no longer exists)\n",
        );
    }

    #[test]
    fn applied_summary_uses_preview_framing_when_not_written() {
        let plan = applied(&[("home-manager.nixpkgs", "nixpkgs")], &["crane.old"]);
        let out = applied_summary(&plan, false, true, &[], &InputSizes::default());
        assert_eq!(
            out,
            "Would deduplicate 1 input (size unknown).\n\
             \x20 home-manager.nixpkgs -> nixpkgs (size unknown)\n\
             Would remove 1 stale follows declaration.\n\
             \x20 crane.old (input no longer exists)\n",
        );
    }

    #[test]
    fn applied_summary_excludes_lock_routed_entries_from_realized() {
        // `crane.nixpkgs` is already deduplicated,
        // application must not count toward realized savings.
        let plan = applied(
            &[
                ("home-manager.nixpkgs", "nixpkgs"),
                ("crane.nixpkgs", "nixpkgs"),
            ],
            &[],
        );
        let nested_inputs = [nested("crane.nixpkgs", Some("nixpkgs"))];
        let sizes = InputSizes::from_attr_entries([
            (
                "home-manager.nixpkgs".to_string(),
                "/store/a".to_string(),
                Some(100 * 1024 * 1024),
            ),
            (
                "crane.nixpkgs".to_string(),
                "/store/a".to_string(),
                Some(100 * 1024 * 1024),
            ),
        ]);
        let out = applied_summary(&plan, true, true, &nested_inputs, &sizes);
        assert_eq!(
            out,
            "Deduplicated 1 input (100 MiB saved).\n\
             \x20 home-manager.nixpkgs -> nixpkgs (100 MiB)\n",
        );
    }

    #[test]
    fn applied_summary_keeps_retargeted_entries_in_realized() {
        let plan = applied(&[("crane.nixpkgs", "nixpkgs")], &[]);
        let nested_inputs = [nested("crane.nixpkgs", Some("flake-utils.nixpkgs"))];
        let out = applied_summary(&plan, true, true, &nested_inputs, &InputSizes::default());
        assert_eq!(
            out,
            "Deduplicated 1 input (size unknown).\n\
             \x20 crane.nixpkgs -> nixpkgs (size unknown)\n",
        );
    }

    #[test]
    fn existing_summary_saturates_absurd_sizes() {
        let nested_inputs = [
            nested("a.nixpkgs", Some("nixpkgs")),
            nested("b.nixpkgs", Some("nixpkgs")),
        ];
        let sizes = InputSizes::from_attr_entries([
            (
                "a.nixpkgs".to_string(),
                "/store/a".to_string(),
                Some(u64::MAX),
            ),
            ("b.nixpkgs".to_string(), "/store/a".to_string(), None),
        ]);
        let out = existing_summary(&nested_inputs, &sizes).expect("has follows");
        assert_eq!(
            out,
            "Already deduplicated by existing follows: 2 inputs across 1 target \
             (~17179869184.0 GiB).\n\
             \x20 a.nixpkgs (~17179869184.0 GiB) x 2 (~17179869184.0 GiB)\n",
        );
    }

    #[test]
    fn existing_summary_groups_followers_by_store_path() {
        let nested_inputs = [
            nested("a.nixpkgs", Some("nixpkgs")),
            nested("b.nixpkgs", Some("nixpkgs")),
            nested("a.flake-utils", Some("flake-utils")),
        ];
        let mib = 1024 * 1024;
        let sizes = InputSizes::from_attr_entries([
            (
                "nixpkgs".to_string(),
                "/store/a".to_string(),
                Some(200 * mib),
            ),
            ("a.nixpkgs".to_string(), "/store/a".to_string(), None),
            ("b.nixpkgs".to_string(), "/store/a".to_string(), None),
            (
                "a.flake-utils".to_string(),
                "/store/b".to_string(),
                Some(10 * mib),
            ),
        ]);
        let out = existing_summary(&nested_inputs, &sizes).expect("has follows");
        assert_eq!(
            out,
            "Already deduplicated by existing follows: 3 inputs across 2 targets (~410 MiB).\n\
             \x20 nixpkgs (~200 MiB) x 2 (~400 MiB)\n\
             \x20 a.flake-utils (~10 MiB)\n",
        );
    }

    #[test]
    fn existing_summary_keeps_unknown_targets_apart() {
        // Two unresolved followers of different targets must not merge into one row.
        let nested_inputs = [
            nested("a.nixpkgs", Some("nixpkgs")),
            nested("a.flake-utils", Some("flake-utils")),
        ];
        let out = existing_summary(&nested_inputs, &InputSizes::default()).expect("has follows");
        assert_eq!(
            out,
            "Already deduplicated by existing follows: 2 inputs across 2 targets \
             (size unknown).\n\
             \x20 flake-utils (size unknown)\n\
             \x20 nixpkgs (size unknown)\n",
        );
    }

    #[test]
    fn existing_summary_is_none_without_follows() {
        let nested_inputs = [nested("a.nixpkgs", None)];
        assert!(existing_summary(&nested_inputs, &InputSizes::default()).is_none());
    }
}
