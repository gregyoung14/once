use std::path::Path;

use super::plan::InputPlan;
use crate::remote::archive::limits::{limit_error, LimitedWriter};
use crate::remote::archive::{archive_error, file_error, TempArchive};
use crate::remote::path::relative_link_stays_within;
use crate::Result;

pub(super) fn archive(
    workspace_root: &Path,
    plan: InputPlan,
    provider: &'static str,
) -> Result<TempArchive> {
    let parent = workspace_root.join(".once/tmp");
    std::fs::create_dir_all(&parent).map_err(|source| {
        file_error(
            "create_remote_archive_directory",
            Path::new(".once/tmp"),
            source,
        )
    })?;
    let file = tempfile::NamedTempFile::new_in(parent).map_err(|source| {
        file_error(
            "create_remote_input_archive",
            Path::new(".once/tmp"),
            source,
        )
    })?;
    let mut builder = tar::Builder::new(LimitedWriter::new(file, plan.archive_bytes));
    builder.follow_symlinks(false);
    for (relative, source) in plan.entries {
        if relative.as_os_str().is_empty() {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&source)
            .map_err(|source_error| file_error("read_remote_input", &relative, source_error))?;
        if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(&source).map_err(|source_error| {
                file_error("read_remote_input_link", &relative, source_error)
            })?;
            if !relative_link_stays_within(&relative, &target, &plan.declared_roots) {
                return Err(archive_error(
                    provider,
                    format!(
                        "input symbolic link `{}` targets `{}` outside declared input trees",
                        relative.display(),
                        target.display()
                    ),
                ));
            }
        }
        let result = if metadata.is_dir() && !metadata.file_type().is_symlink() {
            builder.append_dir(&relative, &source)
        } else if metadata.is_file() || metadata.file_type().is_symlink() {
            builder.append_path_with_name(&source, &relative)
        } else {
            return Err(archive_error(
                provider,
                format!(
                    "input `{}` is not a file, directory, or symbolic link",
                    relative.display()
                ),
            ));
        };
        result.map_err(|source_error| {
            limit_error(provider, &source_error)
                .unwrap_or_else(|| file_error("archive_remote_input", &relative, source_error))
        })?;
    }
    builder.finish().map_err(|source| {
        limit_error(provider, &source).unwrap_or_else(|| {
            file_error(
                "finish_remote_input_archive",
                Path::new(".once/tmp"),
                source,
            )
        })
    })?;
    let file = builder
        .into_inner()
        .map_err(|source| {
            file_error(
                "finish_remote_input_archive",
                Path::new(".once/tmp"),
                source,
            )
        })?
        .into_inner();
    let len = file
        .as_file()
        .metadata()
        .map_err(|source| file_error("read_remote_input_archive", Path::new(".once/tmp"), source))?
        .len();
    Ok(TempArchive {
        path: file.into_temp_path(),
        len,
    })
}
