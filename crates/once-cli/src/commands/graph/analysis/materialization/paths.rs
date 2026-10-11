use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use once_cas::ActionResult;
use once_core::WorkspacePath;

pub(super) fn batch_len(results: &[ActionResult], workspace: &Path, limit: usize) -> usize {
    // Missing Windows paths can acquire short-name aliases only after creation.
    if cfg!(windows) || results.len() == 1 || limit <= 1 {
        return 1;
    }
    let mut paths = BTreeSet::new();
    let mut count = 0;
    for result in results.iter().take(limit.max(1)) {
        let Some(footprint) = footprint(result, workspace) else {
            break;
        };
        if footprint.iter().any(|path| overlaps(&paths, path)) {
            break;
        }
        paths.extend(footprint);
        count += 1;
    }
    count.max(1)
}

fn overlaps(paths: &BTreeSet<PathBuf>, candidate: &Path) -> bool {
    candidate.ancestors().any(|parent| paths.contains(parent))
        || paths
            .range(candidate.to_path_buf()..)
            .next()
            .is_some_and(|path| path.starts_with(candidate))
}

pub(super) fn disjoint(
    workspace: &Path,
    left: &BTreeSet<String>,
    right: &BTreeSet<String>,
) -> bool {
    if left.is_empty() || right.is_empty() {
        return true;
    }
    if cfg!(windows) {
        return false;
    }
    let Some(left) = footprint_paths(left.iter().map(String::as_str), workspace) else {
        return false;
    };
    let Some(right) = footprint_paths(right.iter().map(String::as_str), workspace) else {
        return false;
    };
    let left = left.into_iter().collect();
    right.iter().all(|path| !overlaps(&left, path))
}

fn footprint(result: &ActionResult, workspace: &Path) -> Option<Vec<PathBuf>> {
    footprint_paths(result.outputs.keys().map(String::as_str), workspace)
}

fn footprint_paths<'a>(
    outputs: impl Iterator<Item = &'a str>,
    workspace: &Path,
) -> Option<Vec<PathBuf>> {
    let mut paths = BTreeSet::new();
    for output in outputs {
        let logical = WorkspacePath::try_from(output).ok()?.resolve(workspace);
        let canonical = resolve_existing_ancestors(&logical)?;
        let logical = logical.to_str()?;
        let canonical = canonical.to_str()?;
        if !logical.is_ascii() || !canonical.is_ascii() {
            return None;
        }
        paths.insert(PathBuf::from(logical.to_ascii_lowercase()));
        paths.insert(PathBuf::from(canonical.to_ascii_lowercase()));
    }
    Some(paths.into_iter().collect())
}

fn resolve_existing_ancestors(path: &Path) -> Option<PathBuf> {
    let mut ancestor = path;
    let mut suffix = Vec::new();
    loop {
        match ancestor.canonicalize() {
            Ok(mut canonical) => {
                for component in suffix.iter().rev() {
                    canonical.push(component);
                }
                return Some(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match ancestor.symlink_metadata() {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    _ => return None,
                }
                suffix.push(ancestor.file_name()?.to_os_string());
                ancestor = ancestor.parent()?;
            }
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use once_cas::Digest;
    use std::collections::BTreeMap;

    fn result(path: &str) -> ActionResult {
        ActionResult {
            exit_code: 0,
            stdout: None,
            stderr: None,
            outputs: BTreeMap::from([(path.into(), Digest::of_bytes(path.as_bytes()))]),
        }
    }

    #[test]
    fn independent_paths_are_bounded_and_shared_paths_are_barriers() {
        let workspace = tempfile::tempdir().unwrap();
        let independent = if cfg!(windows)
            || !workspace.path().to_string_lossy().is_ascii()
            || !workspace
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .is_ascii()
        {
            1
        } else {
            2
        };
        assert_eq!(
            batch_len(
                &[result("a"), result("b"), result("c")],
                workspace.path(),
                2
            ),
            independent
        );
        for (first, second) in [
            ("a", "a"),
            ("a", "a/child"),
            ("a/child", "a"),
            ("a", "A"),
            ("a", "./a"),
            ("", "a"),
        ] {
            assert_eq!(
                batch_len(&[result(first), result(second)], workspace.path(), 4),
                1
            );
        }
        assert_eq!(
            batch_len(&[result("a"), result("a-other")], workspace.path(), 4),
            independent
        );
    }

    #[test]
    fn unicode_names_serialize_when_alias_rules_are_not_proven() {
        let workspace = tempfile::tempdir().unwrap();
        assert_eq!(
            batch_len(
                &[result("résumé"), result("re\u{301}sume\u{301}")],
                workspace.path(),
                4
            ),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_parents_are_uncertain_even_before_target_creation() {
        let workspace = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("tree", workspace.path().join("alias")).unwrap();
        assert!(!disjoint(
            workspace.path(),
            &BTreeSet::from(["tree".into()]),
            &BTreeSet::from(["alias/child".into()])
        ));
        assert_eq!(
            batch_len(
                &[result("tree"), result("alias/child")],
                workspace.path(),
                4
            ),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn existing_symlink_parents_are_barriers_even_for_missing_outputs() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir(workspace.path().join("tree")).unwrap();
        std::os::unix::fs::symlink("tree", workspace.path().join("alias")).unwrap();
        assert_eq!(
            batch_len(
                &[result("tree/child"), result("alias/child")],
                workspace.path(),
                4
            ),
            1
        );
        assert_eq!(
            batch_len(
                &[result("tree"), result("alias/child")],
                workspace.path(),
                4
            ),
            1
        );
        assert_eq!(
            batch_len(
                &[result("alias"), result("alias/child")],
                workspace.path(),
                4
            ),
            1
        );
    }
}
