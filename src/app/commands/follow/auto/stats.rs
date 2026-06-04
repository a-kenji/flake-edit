//! Rendering for `--stats` and `--dry-run` output.
//!
//! Sizes come from [`InputSizes::for_flake`], which makes two `nix`
//! invocations once per command and produces a map keyed by attribute
//! path. The renderer looks up each follow entry by its source
//! attribute path; there is no per-follow URL construction and no
//! per-follow `nix` invocation.
//!
//! Two groups feed the output:
//!
//! - **Realized**: follows the pass just applied (or would apply, under
//!   `--dry-run`). Each line costs one would-be-duplicated copy of the
//!   underlying input.
//! - **Already deduplicated by lock**: follows that `flake.lock` already
//!   routes, with or without a source declaration. Each line represents
//!   disk space the user is already saving via Nix's store dedup.
//!
//! Per-line sizes show how much that one follow contributes; the header
//! sums them. With many follows pointing at the same input (e.g.
//! nixpkgs), the math `N * size` is intentional: each follow is one
//! duplicate copy that store dedup folds together.

use std::collections::{BTreeMap, HashSet};

use crate::follows::AttrPath;
use crate::lock::{NestedInput, sizes::InputSizes};

use super::AppliedPlan;

const BYTES_PER_MIB: u64 = 1024 * 1024;

/// Render the realized-savings group.
///
/// Entries whose lock state already routes the follow via upstream
/// propagation are excluded from the realized count: the source-side
/// declaration is still added, but the disk saving has already been
/// realized by Nix. Those entries reappear under
/// [`render_already_deduplicated`].
pub(super) fn render_realized_summary(
    applied: &AppliedPlan,
    dry_run: bool,
    stats: bool,
    nested_inputs: &[NestedInput],
    sizes: &InputSizes,
) {
    let lock_routed: HashSet<AttrPath> = if stats {
        nested_inputs
            .iter()
            .filter(|n| n.follows.is_some())
            .map(|n| n.path.clone())
            .collect()
    } else {
        HashSet::new()
    };

    let realized: Vec<usize> = (0..applied.applied_follows.len())
        .filter(|i| !lock_routed.contains(&applied.applied_follows[*i].0))
        .collect();

    if !realized.is_empty() {
        let count = realized.len();
        let header_verb = if dry_run {
            "Would deduplicate"
        } else {
            "Deduplicated"
        };
        let noun = if count == 1 { "input" } else { "inputs" };

        let per_line: Vec<Option<u64>> = if stats {
            realized
                .iter()
                .map(|i| sizes.for_attr_path(&applied.applied_follows[*i].0))
                .collect()
        } else {
            vec![None; count]
        };

        if stats {
            println!(
                "{header_verb} {count} {noun}{}.",
                summary_suffix(&per_line, false)
            );
        } else {
            println!("{header_verb} {count} {noun}.");
        }
        for (i, size) in realized.iter().zip(per_line.iter()) {
            let (input_path, target) = &applied.applied_follows[*i];
            println!(
                "  {} -> {}{}",
                input_path,
                target,
                per_input_suffix(*size, false, stats)
            );
        }
    }

    if !applied.unfollowed.is_empty() {
        let count = applied.unfollowed.len();
        let noun = if count == 1 {
            "declaration"
        } else {
            "declarations"
        };
        let header_verb = if dry_run { "Would remove" } else { "Removed" };
        println!("{header_verb} {count} stale follows {noun}.");
        for path in &applied.unfollowed {
            println!("  {} (input no longer exists)", path);
        }
    }
}

/// Render the savings breakdown for follows the lock already routes.
///
/// Follows are grouped by the store path their target resolves to, so
/// 25 followers of nixpkgs collapse to one row labelled
/// `nixpkgs (~191 MiB)  x 25  (~4.6 GiB saved)` instead of 25
/// near-identical lines. The savings model is the user's: `N * size`,
/// since each follower is one would-be-duplicate copy that Nix's store
/// dedup folds together.
///
/// The header reads "Estimated savings: ..." rather than "Already
/// deduplicated by lock: ..." so it doesn't echo the caller's preamble
/// line when there's nothing to apply.
pub(super) fn render_already_deduplicated(nested_inputs: &[NestedInput], sizes: &InputSizes) {
    let entries: Vec<(&AttrPath, &AttrPath)> = nested_inputs
        .iter()
        .filter_map(|n| n.follows.as_ref().map(|f| (&n.path, f)))
        .collect();

    if entries.is_empty() {
        return;
    }

    // Group followers by the store path their attribute resolves to.
    // Followers whose store path is unknown stay separate (keyed by
    // `None`) so we can still show them as best we can.
    let mut groups: BTreeMap<Option<String>, Group> = BTreeMap::new();
    for (path, target) in &entries {
        let store_path = sizes.store_path_for_attr_path(path).map(str::to_string);
        let label_for_group = store_path
            .as_deref()
            .and_then(|sp| sizes.shortest_attr_for_store_path(sp))
            .map(str::to_string);
        let entry = groups.entry(store_path).or_insert_with(|| Group {
            label_attr: label_for_group.clone(),
            target: target.to_string(),
            count: 0,
            unit_size: sizes.for_attr_path(path),
        });
        entry.count += 1;
        if entry.label_attr.is_none() {
            entry.label_attr = label_for_group;
        }
        if entry.unit_size.is_none() {
            entry.unit_size = sizes.for_attr_path(path);
        }
    }

    let total: u64 = groups
        .values()
        .filter_map(|g| g.unit_size.map(|s| s * g.count as u64))
        .sum();
    let any_known = groups.values().any(|g| g.unit_size.is_some());
    let total_count = entries.len();
    let unique = groups.len();
    let follow_noun = if total_count == 1 {
        "follow"
    } else {
        "follows"
    };
    let target_noun = if unique == 1 { "target" } else { "targets" };

    let suffix = if any_known {
        format!(" (~{} saved)", format_size(total))
    } else {
        " (size unknown)".to_string()
    };

    println!();
    println!(
        "Estimated savings: {total_count} {follow_noun} across {unique} {target_noun}{suffix}."
    );

    // Sort groups for stable, readable output: biggest contributor
    // first (count * unit_size), then by label.
    let mut rows: Vec<(&Option<String>, &Group)> = groups.iter().collect();
    rows.sort_by(|(_, a), (_, b)| {
        let a_total = a.unit_size.unwrap_or(0) * a.count as u64;
        let b_total = b.unit_size.unwrap_or(0) * b.count as u64;
        b_total
            .cmp(&a_total)
            .then_with(|| a.display_label().cmp(b.display_label()))
    });

    for (_, group) in rows {
        let label = group.display_label();
        let unit = group
            .unit_size
            .map(|s| format!("~{}", format_size(s)))
            .unwrap_or_else(|| "size unknown".to_string());
        let group_total = group
            .unit_size
            .map(|s| format!(" (~{} saved)", format_size(s * group.count as u64)))
            .unwrap_or_default();
        if group.count == 1 {
            println!("  {label} ({unit})");
        } else {
            println!(
                "  {label} ({unit})  x {count}{group_total}",
                count = group.count
            );
        }
    }
}

/// One row in the grouped-by-target rendering. `label_attr` is the
/// shortest attribute path that maps to this store path (typically the
/// top-level input the others follow into). `target` is kept as a
/// fallback when no attribute label is available.
struct Group {
    label_attr: Option<String>,
    target: String,
    count: usize,
    unit_size: Option<u64>,
}

impl Group {
    fn display_label(&self) -> &str {
        self.label_attr.as_deref().unwrap_or(&self.target)
    }
}

/// Trailing `" (… saved)"` or `" (size unknown)"` for a group header.
fn summary_suffix(sizes: &[Option<u64>], hypothetical: bool) -> String {
    let total: u64 = sizes.iter().filter_map(|s| *s).sum();
    let any_known = sizes.iter().any(|s| s.is_some());
    if !any_known {
        return " (size unknown)".to_string();
    }
    if hypothetical {
        format!(" (~{})", format_size(total))
    } else {
        format!(" ({} saved)", format_size(total))
    }
}

/// Per-input size suffix; empty when stats are off or the size is
/// unknown so the line stays terse.
fn per_input_suffix(size: Option<u64>, hypothetical: bool, stats: bool) -> String {
    if !stats {
        return String::new();
    }
    match size {
        Some(bytes) => {
            let formatted = format_size(bytes);
            if hypothetical {
                format!(" (~{})", formatted)
            } else {
                format!(" ({})", formatted)
            }
        }
        None => String::new(),
    }
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * BYTES_PER_MIB {
        let gib = bytes as f64 / (1024.0 * BYTES_PER_MIB as f64);
        format!("{:.1} GiB", gib)
    } else if bytes >= BYTES_PER_MIB {
        let mib = bytes / BYTES_PER_MIB;
        format!("{} MiB", mib)
    } else if bytes >= 1024 {
        let kib = bytes / 1024;
        format!("{} KiB", kib)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn summary_suffix_marks_unknown_when_no_sizes() {
        assert_eq!(summary_suffix(&[None, None], false), " (size unknown)");
        assert_eq!(summary_suffix(&[None, None], true), " (size unknown)");
    }

    #[test]
    fn summary_suffix_uses_tilde_for_hypothetical() {
        let sizes = [Some(100 * 1024 * 1024), Some(50 * 1024 * 1024)];
        assert_eq!(summary_suffix(&sizes, false), " (150 MiB saved)");
        assert_eq!(summary_suffix(&sizes, true), " (~150 MiB)");
    }
}
