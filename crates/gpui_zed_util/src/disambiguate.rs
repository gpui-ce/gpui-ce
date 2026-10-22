use std::collections::HashMap;
use std::hash::Hash;

/// Finds the minimum detail level that distinguishes each item's description.
/// Unique descriptions stay at level 0. Collisions increase the level until
/// resolved or the description stops changing, preventing loops for identical items.
///
/// `get_description` must eventually stop changing as `detail` increases.
/// Once two consecutive levels produce the same description, the function assumes
/// later levels will also match and no longer checks that item for collisions.
pub fn compute_disambiguation_details<T, D>(
    items: &[T],
    get_description: impl Fn(&T, usize) -> D,
) -> Vec<usize>
where
    D: Eq + Hash + Clone,
{
    let mut details = vec![0usize; items.len()];
    let mut descriptions: HashMap<D, Vec<usize>> = HashMap::default();
    let mut current_descriptions: Vec<D> =
        items.iter().map(|item| get_description(item, 0)).collect();

    loop {
        let mut any_collisions = false;

        for (index, (item, &detail)) in items.iter().zip(&details).enumerate() {
            if detail > 0 {
                let new_description = get_description(item, detail);
                if new_description == current_descriptions[index] {
                    continue;
                }
                current_descriptions[index] = new_description;
            }
            descriptions
                .entry(current_descriptions[index].clone())
                .or_default()
                .push(index);
        }

        for (_, indices) in descriptions.drain() {
            if indices.len() > 1 {
                any_collisions = true;
                for index in indices {
                    details[index] += 1;
                }
            }
        }

        if !any_collisions {
            break;
        }
    }

    details
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_conflicts() {
        let items = vec!["alpha", "beta", "gamma"];
        let details = compute_disambiguation_details(&items, |item, _detail| item.to_string());
        assert_eq!(details, vec![0, 0, 0]);
    }

    #[test]
    fn test_simple_two_way_conflict() {
        // Two items with the same base name but different parents.
        let items = vec![("src/foo.rs", "foo.rs"), ("lib/foo.rs", "foo.rs")];
        let details = compute_disambiguation_details(&items, |item, detail| match detail {
            0 => item.1.to_string(),
            _ => item.0.to_string(),
        });
        assert_eq!(details, vec![1, 1]);
    }

    #[test]
    fn test_three_way_conflict() {
        let items = vec![
            ("foo.rs", "a/foo.rs"),
            ("foo.rs", "b/foo.rs"),
            ("foo.rs", "c/foo.rs"),
        ];
        let details = compute_disambiguation_details(&items, |item, detail| match detail {
            0 => item.0.to_string(),
            _ => item.1.to_string(),
        });
        assert_eq!(details, vec![1, 1, 1]);
    }

    #[test]
    fn test_deeper_conflict() {
        // At detail 0, all three show "file.rs".
        // At detail 1, items 0 and 1 both show "src/file.rs", item 2 shows "lib/file.rs".
        // At detail 2, item 0 shows "a/src/file.rs", item 1 shows "b/src/file.rs".
        let items = vec![
            vec!["file.rs", "src/file.rs", "a/src/file.rs"],
            vec!["file.rs", "src/file.rs", "b/src/file.rs"],
            vec!["file.rs", "lib/file.rs", "x/lib/file.rs"],
        ];
        let details = compute_disambiguation_details(&items, |item, detail| {
            let clamped = detail.min(item.len() - 1);
            item[clamped].to_string()
        });
        assert_eq!(details, vec![2, 2, 1]);
    }

    #[test]
    fn test_mixed_conflicting_and_unique() {
        let items = vec![
            ("src/foo.rs", "foo.rs"),
            ("lib/foo.rs", "foo.rs"),
            ("src/bar.rs", "bar.rs"),
        ];
        let details = compute_disambiguation_details(&items, |item, detail| match detail {
            0 => item.1.to_string(),
            _ => item.0.to_string(),
        });
        assert_eq!(details, vec![1, 1, 0]);
    }

    #[test]
    fn test_identical_items_terminates() {
        // All items return the same description at every detail level.
        // The algorithm must terminate rather than looping forever.
        let items = vec!["same", "same", "same"];
        let details = compute_disambiguation_details(&items, |item, _detail| item.to_string());
        // After bumping to 1, the description doesn't change from level 0,
        // so the items are skipped and the loop terminates.
        assert_eq!(details, vec![1, 1, 1]);
    }

    #[test]
    fn test_single_item() {
        let items = vec!["only"];
        let details = compute_disambiguation_details(&items, |item, _detail| item.to_string());
        assert_eq!(details, vec![0]);
    }

    #[test]
    fn test_empty_input() {
        let items: Vec<&str> = vec![];
        let details = compute_disambiguation_details(&items, |item, _detail| item.to_string());
        let expected: Vec<usize> = vec![];
        assert_eq!(details, expected);
    }

    #[test]
    fn test_duplicate_paths_from_multiple_groups() {
        use std::path::Path;

        // The sidebar can repeat /Users/rtfeldman/code/zed across project groups.
        // Deduplicate before disambiguating, or identical paths collide and require
        // full-path detail. Keep the worktree's distinct zed path and roc path.

        fn path_suffix(path: &Path, detail: usize) -> String {
            let mut components: Vec<_> = path
                .components()
                .rev()
                .filter_map(|c| match c {
                    std::path::Component::Normal(s) => Some(s.to_string_lossy()),
                    _ => None,
                })
                .take(detail + 1)
                .collect();
            components.reverse();
            components.join("/")
        }

        let all_paths: Vec<&Path> = vec![
            Path::new("/Users/rtfeldman/code/worktrees/zed/focal-arrow/zed"),
            Path::new("/Users/rtfeldman/code/zed"),
            Path::new("/Users/rtfeldman/code/roc"),
        ];

        let details =
            compute_disambiguation_details(&all_paths, |path, detail| path_suffix(path, detail));

        // focal-arrow/zed and code/zed both end in "zed", so they need detail 1.
        // "roc" is unique at detail 0.
        assert_eq!(details, vec![1, 1, 0]);

        assert_eq!(path_suffix(all_paths[0], details[0]), "focal-arrow/zed");
        assert_eq!(path_suffix(all_paths[1], details[1]), "code/zed");
        assert_eq!(path_suffix(all_paths[2], details[2]), "roc");
    }
}
