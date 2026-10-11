use super::*;
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap();
        let name = format!(
            "tessera-fault-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = root.join(name);
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn dest(&self) -> PathBuf {
        self.0.join("project.tessera")
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn io_error() -> AppError {
    io::Error::other("injected failure").into()
}
fn run(
    t: &Temp,
    c: &JobControl,
    expected: Option<&FileStamp>,
    hook: impl FnMut(Stage, &Path, &Path) -> AppResult<()>,
) -> AppResult<SaveOutcome> {
    publish_with(
        &t.dest(),
        Some(RevisionId(2)),
        expected,
        c,
        |writer| {
            writer.write_all(b"new complete revision")?;
            Ok(())
        },
        hook,
    )
}
#[test]
fn late_races_preserve_frozen_displaced_and_third_destination() {
    let t = Temp::new();
    std::fs::write(t.dest(), b"initial").unwrap();
    let expected = stamp(&t.dest()).unwrap();
    let outcome = run(
        &t,
        &JobControl::default(),
        Some(&expected),
        |stage, dest, _| {
            if stage == Stage::Publish {
                std::fs::write(dest, b"late external complete version")?;
            }
            if stage == Stage::Published {
                let third = dest.with_extension("third");
                std::fs::write(&third, b"third complete version")?;
                std::fs::rename(third, dest)?;
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(outcome.publication, Publication::Conflict);
    assert!(outcome.committed_stamp.is_some());
    let paths = outcome.recovery.unwrap();
    assert_eq!(
        std::fs::read(paths.frozen_revision.unwrap()).unwrap(),
        b"new complete revision"
    );
    assert_eq!(
        std::fs::read(paths.displaced.unwrap()).unwrap(),
        b"late external complete version"
    );
    assert_eq!(std::fs::read(t.dest()).unwrap(), b"third complete version");
}
#[test]
fn late_in_place_overwrite_cannot_mutate_frozen_copy() {
    let t = Temp::new();
    std::fs::write(t.dest(), b"initial").unwrap();
    let expected = stamp(&t.dest()).unwrap();
    let outcome = run(
        &t,
        &JobControl::default(),
        Some(&expected),
        |stage, dest, _| {
            if stage == Stage::Published {
                std::fs::write(dest, b"late in-place writer")?;
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(outcome.publication, Publication::Conflict);
    let paths = outcome.recovery.unwrap();
    assert_eq!(
        std::fs::read(paths.frozen_revision.unwrap()).unwrap(),
        b"new complete revision"
    );
    assert_eq!(std::fs::read(paths.displaced.unwrap()).unwrap(), b"initial");
    assert_eq!(std::fs::read(t.dest()).unwrap(), b"late in-place writer");
}
#[test]
fn cancellation_immediately_before_and_after_publication() {
    for before in [true, false] {
        let t = Temp::new();
        let c = JobControl::default();
        let outcome = run(&t, &c, None, |stage, _, _| {
            if stage
                == if before {
                    Stage::Publish
                } else {
                    Stage::Published
                }
            {
                c.cancel();
            }
            Ok(())
        });
        if before {
            assert!(matches!(outcome, Err(AppError::Cancelled)));
            assert!(!t.dest().exists());
        } else {
            let outcome = outcome.unwrap();
            assert_eq!(outcome.publication, Publication::Committed);
            assert!(outcome.committed_stamp.is_some());
            assert_eq!(std::fs::read(t.dest()).unwrap(), b"new complete revision");
        }
    }
}
#[test]
fn individual_failures_leave_complete_files_and_commit_facts() {
    for fault in [
        Stage::Write,
        Stage::Flush,
        Stage::Publish,
        Stage::Published,
        Stage::Cleanup,
        Stage::DirectorySync,
    ] {
        let t = Temp::new();
        std::fs::write(t.dest(), b"prior complete").unwrap();
        let expected = stamp(&t.dest()).unwrap();
        let result = run(
            &t,
            &JobControl::default(),
            Some(&expected),
            |stage, _, _| {
                if stage == fault {
                    Err(io_error())
                } else {
                    Ok(())
                }
            },
        );
        if matches!(fault, Stage::Write | Stage::Flush | Stage::Publish) {
            assert!(result.is_err());
            assert_eq!(std::fs::read(t.dest()).unwrap(), b"prior complete");
        } else {
            let outcome = result.unwrap();
            assert_eq!(outcome.publication, Publication::Committed);
            assert!(outcome.warning.is_some());
            assert!(outcome.committed_stamp.is_some());
            assert_eq!(std::fs::read(t.dest()).unwrap(), b"new complete revision");
            assert_eq!(
                std::fs::read(outcome.recovery.unwrap().displaced.unwrap()).unwrap(),
                b"prior complete"
            );
        }
    }
}
#[test]
fn unsupported_publication_requires_save_as() {
    let t = Temp::new();
    let result = run(&t, &JobControl::default(), None, |stage, _, _| {
        if stage == Stage::Publish {
            Err(AppError::UnsupportedPublication("Choose Save as.".into()))
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Err(AppError::UnsupportedPublication(_))));
    assert!(!t.dest().exists());
}
#[test]
fn second_preflight_preserves_external_and_our_complete_revision() {
    let t = Temp::new();
    std::fs::write(t.dest(), b"prior").unwrap();
    let expected = stamp(&t.dest()).unwrap();
    let result = run(
        &t,
        &JobControl::default(),
        Some(&expected),
        |stage, dest, _| {
            if stage == Stage::Precheck {
                std::fs::write(dest, b"external")?;
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(result.publication, Publication::Conflict);
    assert!(result.committed_stamp.is_none());
    assert_eq!(std::fs::read(t.dest()).unwrap(), b"external");
    assert_eq!(
        std::fs::read(result.recovery.unwrap().frozen_revision.unwrap()).unwrap(),
        b"new complete revision"
    );
}
#[test]
fn native_no_replace_preserves_late_destination() {
    let t = Temp::new();
    let outcome = run(&t, &JobControl::default(), None, |stage, dest, _| {
        if stage == Stage::Publish {
            std::fs::write(dest, b"unrelated")?;
        }
        Ok(())
    })
    .unwrap();
    assert!(outcome.committed_stamp.is_none());
    assert_eq!(std::fs::read(t.dest()).unwrap(), b"unrelated");
    assert_eq!(
        std::fs::read(outcome.recovery.unwrap().frozen_revision.unwrap()).unwrap(),
        b"new complete revision"
    );
}
#[test]
fn cooperative_writers_wait_and_waiting_is_cancellable() {
    let t = Temp::new();
    let d = Directory::open(&t.0).unwrap();
    let guard = d.lock(&JobControl::default()).unwrap();
    let c = JobControl::default();
    let other = c.clone();
    let path = t.0.clone();
    let thread =
        std::thread::spawn(move || Directory::open(&path).unwrap().lock(&other).map(|_| ()));
    std::thread::sleep(std::time::Duration::from_millis(30));
    c.cancel();
    assert!(matches!(thread.join().unwrap(), Err(AppError::Cancelled)));
    drop(guard);
    assert!(d.lock(&JobControl::default()).is_ok());
}

#[test]
fn reported_native_failure_after_real_publication_retains_committed_facts() {
    let t = Temp::new();
    std::fs::write(t.dest(), b"old complete").unwrap();
    let expected = stamp(&t.dest()).unwrap();
    let result = run(
        &t,
        &JobControl::default(),
        Some(&expected),
        |stage, _, _| {
            if stage == Stage::NativeResult {
                Err(io_error())
            } else {
                Ok(())
            }
        },
    )
    .unwrap();
    assert_eq!(result.publication, Publication::Committed);
    assert!(result.warning.is_some());
    assert_eq!(result.committed_stamp, Some(stamp(&t.dest()).unwrap()));
    let paths = result.recovery.unwrap();
    assert_eq!(
        std::fs::read(paths.displaced.unwrap()).unwrap(),
        b"old complete"
    );
    assert_eq!(
        std::fs::read(paths.frozen_revision.unwrap()).unwrap(),
        b"new complete revision"
    );
}
#[test]
fn ambiguous_native_failure_preserves_every_surviving_version() {
    let t = Temp::new();
    std::fs::write(t.dest(), b"old complete").unwrap();
    let expected = stamp(&t.dest()).unwrap();
    let result = run(
        &t,
        &JobControl::default(),
        Some(&expected),
        |stage, dest, _| {
            if stage == Stage::NativeResult {
                std::fs::write(dest, b"third complete")?;
                Err(io_error())
            } else {
                Ok(())
            }
        },
    )
    .unwrap();
    assert_eq!(result.publication, Publication::Uncertain);
    assert!(result.committed_stamp.is_none());
    assert_eq!(std::fs::read(t.dest()).unwrap(), b"third complete");
    let paths = result.recovery.unwrap();
    assert_eq!(
        std::fs::read(paths.displaced.unwrap()).unwrap(),
        b"old complete"
    );
    assert_eq!(
        std::fs::read(paths.frozen_revision.unwrap()).unwrap(),
        b"new complete revision"
    );
}
#[test]
fn disappearing_destination_during_native_commit_keeps_frozen_revision() {
    let t = Temp::new();
    std::fs::write(t.dest(), b"old complete").unwrap();
    let expected = stamp(&t.dest()).unwrap();
    let result = run(
        &t,
        &JobControl::default(),
        Some(&expected),
        |stage, dest, _| {
            if stage == Stage::Publish {
                std::fs::remove_file(dest)?;
            }
            Ok(())
        },
    )
    .unwrap();
    assert!(result.committed_stamp.is_none());
    assert!(!t.dest().exists());
    assert_eq!(
        std::fs::read(result.recovery.unwrap().frozen_revision.unwrap()).unwrap(),
        b"new complete revision"
    );
}
#[test]
fn partial_encoding_failure_never_publishes_a_prefix() {
    let t = Temp::new();
    std::fs::write(t.dest(), b"old complete").unwrap();
    let expected = stamp(&t.dest()).unwrap();
    let result = publish_with(
        &t.dest(),
        None,
        Some(&expected),
        &JobControl::default(),
        |writer| {
            writer.write_all(b"partial new data")?;
            Err(io_error())
        },
        |_, _, _| Ok(()),
    );
    assert!(result.is_err());
    assert_eq!(std::fs::read(t.dest()).unwrap(), b"old complete");
}
#[test]
fn streaming_ceiling_rejects_before_writing_beyond_limit() {
    let t = Temp::new();
    let c = JobControl::default();
    let file = File::create(t.0.join("bounded")).unwrap();
    let mut writer = BoundedWriter {
        inner: BufWriter::new(file),
        bytes: MAX_PROJECT_BYTES - 1,
        error: None,
        control: &c,
        next_checkpoint: 0,
    };
    assert!(writer.write_all(b"xx").is_err());
    assert!(matches!(
        writer.error,
        Some(AppError::SizeLimit {
            limit: MAX_PROJECT_BYTES
        })
    ));
    assert_eq!(writer.inner.get_ref().metadata().unwrap().len(), 0);
}
#[test]
fn cooperative_writers_reconcile_after_the_first_writer_releases_the_lock() {
    use std::sync::{Arc, Barrier};
    let t = Temp::new();
    std::fs::write(t.dest(), b"original").unwrap();
    let expected = stamp(&t.dest()).unwrap();
    let other_expected = expected.clone();
    let dest = t.dest();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let other_entered = entered.clone();
    let other_release = release.clone();
    let first = std::thread::spawn(move || {
        publish_with(
            &dest,
            None,
            Some(&other_expected),
            &JobControl::default(),
            |writer| {
                writer.write_all(b"first writer")?;
                Ok(())
            },
            |stage, _, _| {
                if stage == Stage::Publish {
                    other_entered.wait();
                    other_release.wait();
                }
                Ok(())
            },
        )
    });
    entered.wait();
    let dest = t.dest();
    let second = std::thread::spawn(move || {
        publish(
            &dest,
            None,
            Some(&expected),
            &JobControl::default(),
            |writer| {
                writer.write_all(b"second writer")?;
                Ok(())
            },
        )
    });
    release.wait();
    assert_eq!(
        first.join().unwrap().unwrap().publication,
        Publication::Committed
    );
    assert!(matches!(second.join().unwrap(), Err(AppError::Conflict(_))));
    assert_eq!(std::fs::read(t.dest()).unwrap(), b"first writer");
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn unsafe_lock_and_permission_failure_preserve_destination() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let t = Temp::new();
    std::fs::write(t.dest(), b"keep").unwrap();
    let expected = stamp(&t.dest()).unwrap();
    symlink(t.dest(), t.0.join(".tessera-publication.lock")).unwrap();
    assert!(run(
        &t,
        &JobControl::default(),
        Some(&expected),
        |_, _, _| Ok(())
    )
    .is_err());
    assert_eq!(std::fs::read(t.dest()).unwrap(), b"keep");
    std::fs::remove_file(t.0.join(".tessera-publication.lock")).unwrap();
    std::fs::set_permissions(&t.0, std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = run(
        &t,
        &JobControl::default(),
        Some(&expected),
        |_, _, _| Ok(()),
    );
    std::fs::set_permissions(&t.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.is_err());
    assert_eq!(std::fs::read(t.dest()).unwrap(), b"keep");
}
#[cfg(target_os = "windows")]
#[test]
fn windows_locked_destination_preserves_all_complete_versions() {
    use std::os::windows::fs::OpenOptionsExt;
    let t = Temp::new();
    std::fs::write(t.dest(), b"old complete").unwrap();
    let expected = stamp(&t.dest()).unwrap();
    let _locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
        .open(t.dest())
        .unwrap();
    let result = run(
        &t,
        &JobControl::default(),
        Some(&expected),
        |_, _, _| Ok(()),
    )
    .unwrap();
    assert!(result.committed_stamp.is_none());
    assert_eq!(std::fs::read(t.dest()).unwrap(), b"old complete");
    assert_eq!(
        std::fs::read(result.recovery.unwrap().frozen_revision.unwrap()).unwrap(),
        b"new complete revision"
    );
}

#[test]
fn failed_native_replace_labels_new_prepared_file_without_calling_it_displaced() {
    let t = Temp::new();
    std::fs::write(t.dest(), b"prior").unwrap();
    let expected = stamp(&t.dest()).unwrap();
    let outcome = run(
        &t,
        &JobControl::default(),
        Some(&expected),
        |stage, dest, _| {
            if stage == Stage::Publish {
                std::fs::remove_file(dest)?;
            }
            Ok(())
        },
    )
    .unwrap();
    let paths = outcome.recovery.unwrap();
    assert!(paths.displaced.is_none());
    assert_eq!(
        std::fs::read(paths.prepared.unwrap()).unwrap(),
        b"new complete revision"
    );
}
#[test]
fn publication_cannot_replace_its_cooperative_lock() {
    let t = Temp::new();
    let dir = Directory::open(&t.0).unwrap();
    drop(dir.lock(&JobControl::default()).unwrap());
    let path = t.0.join(".tessera-publication.lock");
    let expected = stamp(&path).unwrap();
    let result = publish(
        &path,
        None,
        Some(&expected),
        &JobControl::default(),
        |writer| {
            writer.write_all(b"bad lock replacement")?;
            Ok(())
        },
    );
    assert!(result.is_err());
    assert_eq!(stamp(&path).unwrap(), expected);
}

#[test]
fn late_external_version_is_reported_even_when_native_api_returns_an_error() {
    let t = Temp::new();
    std::fs::write(t.dest(), b"old complete").unwrap();
    let expected = stamp(&t.dest()).unwrap();
    let result = run(
        &t,
        &JobControl::default(),
        Some(&expected),
        |stage, dest, _| {
            if stage == Stage::Publish {
                std::fs::write(dest, b"external complete")?;
            }
            if stage == Stage::NativeResult {
                return Err(io_error());
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(result.publication, Publication::Conflict);
    assert!(result.committed_stamp.is_some());
    assert_eq!(
        std::fs::read(result.recovery.unwrap().displaced.unwrap()).unwrap(),
        b"external complete"
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn renamed_parent_returns_verified_destination_and_displaced_paths() {
    for race in [
        Stage::Precheck,
        Stage::Publish,
        Stage::Published,
        Stage::Cleanup,
        Stage::DirectorySync,
    ] {
        let t = Temp::new();
        let original = t.0.join("original");
        let moved = t.0.join("moved");
        std::fs::create_dir(&original).unwrap();
        let path = original.join("project.tessera");
        std::fs::write(&path, b"prior complete").unwrap();
        let expected = stamp(&path).unwrap();
        let outcome = publish_with(
            &path,
            Some(RevisionId(2)),
            Some(&expected),
            &JobControl::default(),
            |writer| {
                writer.write_all(b"new complete revision")?;
                Ok(())
            },
            |stage, _, _| {
                if stage == race {
                    std::fs::rename(&original, &moved)?;
                    std::fs::create_dir(&original)?;
                    std::fs::write(original.join("project.tessera"), b"unrelated complete")?;
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(outcome.publication, Publication::Committed, "{race:?}");
        assert_eq!(
            outcome.destination,
            moved.join("project.tessera"),
            "{race:?}"
        );
        assert_eq!(
            std::fs::read(&outcome.destination).unwrap(),
            b"new complete revision"
        );
        assert_eq!(
            outcome.committed_stamp,
            Some(stamp(&outcome.destination).unwrap())
        );
        let paths = outcome.recovery.unwrap();
        assert!(paths.directory.starts_with(&moved));
        assert_eq!(
            std::fs::read(paths.displaced.unwrap()).unwrap(),
            b"prior complete"
        );
        assert_eq!(std::fs::read(path).unwrap(), b"unrelated complete");
    }
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn unverifiable_parent_location_preserves_frozen_revision_and_commit_fact() {
    use std::os::unix::fs::PermissionsExt;
    for race in [Stage::Published, Stage::Cleanup, Stage::DirectorySync] {
        let t = Temp::new();
        std::fs::write(t.dest(), b"prior complete").unwrap();
        let expected = stamp(&t.dest()).unwrap();
        let result = run(
            &t,
            &JobControl::default(),
            Some(&expected),
            |stage, _, _| {
                if stage == race {
                    std::fs::set_permissions(&t.0, std::fs::Permissions::from_mode(0o000))?;
                }
                Ok(())
            },
        );
        std::fs::set_permissions(&t.0, std::fs::Permissions::from_mode(0o700)).unwrap();
        let outcome = result.unwrap();
        assert_eq!(outcome.publication, Publication::Uncertain, "{race:?}");
        assert!(outcome.committed_stamp.is_some());
        assert!(outcome.warning.as_ref().unwrap().contains("last-known"));
        let paths = outcome.recovery.unwrap();
        assert_eq!(
            std::fs::read(paths.frozen_revision.unwrap()).unwrap(),
            b"new complete revision"
        );
        assert_eq!(
            std::fs::read(paths.displaced.unwrap()).unwrap(),
            b"prior complete"
        );
        assert_eq!(std::fs::read(t.dest()).unwrap(), b"new complete revision");
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn project_read_rejects_relocated_parent_without_returning_a_project() {
    let t = Temp::new();
    let original = t.0.join("original");
    let moved = t.0.join("moved");
    std::fs::create_dir(&original).unwrap();
    let path = original.join("project.tessera");
    let project = Project::new("Opened project", Dataset::new(vec![]).unwrap()).unwrap();
    let bytes = {
        let mut bytes = Vec::new();
        project_io::encode(&project, &mut bytes, &JobControl::default()).unwrap();
        bytes
    };
    std::fs::write(&path, &bytes).unwrap();
    let result = read_project_checked(&path, &JobControl::default(), &[], 0, || {
        std::fs::rename(&original, &moved)?;
        std::fs::create_dir(&original)?;
        std::fs::write(&path, b"unrelated complete")?;
        Ok(())
    });
    assert!(matches!(result, Err(AppError::Conflict(_))));
    assert_eq!(std::fs::read(&path).unwrap(), b"unrelated complete");
    assert_eq!(std::fs::read(moved.join("project.tessera")).unwrap(), bytes);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn traversal_only_ancestor_preserves_final_parent_publication_and_sync() {
    use std::os::unix::fs::PermissionsExt;
    let t = Temp::new();
    let chosen = t.0.join("chosen-folder");
    std::fs::create_dir(&chosen).unwrap();
    let destination = chosen.join("project.tessera");
    // The owner may traverse this ancestor but may not read/list its contents.
    std::fs::set_permissions(&t.0, std::fs::Permissions::from_mode(0o100)).unwrap();
    let listing = std::fs::read_dir(&t.0);
    let final_parent = Directory::open(&t.0);
    let result = Directory::open(&chosen).and_then(|directory| {
        directory.verify_path(&chosen)?;
        directory.sync()?;
        let current = directory.current_path()?;
        let outcome = publish(&destination, None, None, &JobControl::default(), |writer| {
            writer.write_all(b"complete selected revision")?;
            Ok(())
        })?;
        Ok((current, outcome))
    });
    // Restore permissions before assertions so a failed RED run also cleans up.
    std::fs::set_permissions(&t.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(listing.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert!(matches!(final_parent, Err(AppError::Io(error))
        if error.kind() == io::ErrorKind::PermissionDenied));
    let (current, outcome) = result.expect("traversal must not require ancestor directory reads");
    assert_eq!(current, chosen);
    assert_eq!(outcome.publication, Publication::Committed);
    assert_eq!(
        std::fs::read(destination).unwrap(),
        b"complete selected revision"
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn directory_traversal_keeps_symlink_parent_and_root_boundaries() {
    use std::os::unix::fs::symlink;
    let t = Temp::new();
    let real = t.0.join("real");
    std::fs::create_dir_all(real.join("child")).unwrap();
    let linked = t.0.join("linked");
    symlink(&real, &linked).unwrap();
    assert!(Directory::open(&linked.join("child")).is_err());
    assert!(Directory::open(&linked).is_err());
    assert!(Directory::open(&real.join("../real")).is_err());
    let root = Directory::open(Path::new("/")).unwrap();
    root.verify_path(Path::new("/")).unwrap();
    assert_eq!(root.current_path().unwrap(), Path::new("/"));
}

#[test]
fn prepared_validation_cancellation_precedes_publication_and_cleans_staging() {
    let temp = Temp::new();
    std::fs::write(temp.dest(), b"external complete version").unwrap();
    let expected = stamp(&temp.dest()).unwrap();
    let control = JobControl::default();
    let result = publish_validated(
        &temp.dest(),
        Some(RevisionId(2)),
        Some(&expected),
        &control,
        |writer| {
            writer.write_all(b"new complete revision")?;
            Ok(())
        },
        |file| {
            let mut complete = String::new();
            file.read_to_string(&mut complete)?;
            assert_eq!(complete, "new complete revision");
            control.cancel();
            control.checkpoint(0, None)
        },
        |stage, _, _| {
            assert!(!matches!(
                stage,
                Stage::Precheck | Stage::Publish | Stage::Published
            ));
            Ok(())
        },
    );
    assert!(matches!(result, Err(AppError::Cancelled)));
    assert_eq!(
        std::fs::read(temp.dest()).unwrap(),
        b"external complete version"
    );
    assert_eq!(stamp(&temp.dest()).unwrap(), expected);
    assert!(std::fs::read_dir(&temp.0).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".tessera-recovery-")));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_atomic_no_replace_and_exchange_preserve_both_revisions() {
    let temp = Temp::new();
    let parent = Directory::open(&temp.0).unwrap();
    let tx = parent.create_dir(OsStr::new("transaction")).unwrap();
    native::probe(&tx).unwrap();
    std::fs::write(temp.dest(), b"prior complete").unwrap();
    tx.create(OsStr::new("prepared"))
        .unwrap()
        .write_all(b"new complete")
        .unwrap();
    assert!(native::publish(&parent, &tx, OsStr::new("project.tessera"), false).is_err());
    assert_eq!(std::fs::read(temp.dest()).unwrap(), b"prior complete");
    assert_eq!(
        std::fs::read(tx.path.join("prepared")).unwrap(),
        b"new complete"
    );
    native::publish(&parent, &tx, OsStr::new("project.tessera"), true).unwrap();
    assert_eq!(std::fs::read(temp.dest()).unwrap(), b"new complete");
    assert_eq!(
        std::fs::read(tx.path.join(native::DISPLACED)).unwrap(),
        b"prior complete"
    );
}

#[test]
fn failed_publication_removes_all_owned_probe_names() {
    let temp = Temp::new();
    std::fs::write(temp.dest(), b"prior complete").unwrap();
    let expected = stamp(&temp.dest()).unwrap();
    let mut transaction = None;
    let result = run(
        &temp,
        &JobControl::default(),
        Some(&expected),
        |stage, _, tx| {
            if stage == Stage::Write {
                transaction = Some(tx.to_owned());
                // Reproduce the name left when the Linux exchange probe is unsupported.
                std::fs::write(tx.join("probe-c"), b"probe")?;
                return Err(AppError::UnsupportedPublication(
                    "unsupported exchange".into(),
                ));
            }
            Ok(())
        },
    );
    assert!(matches!(result, Err(AppError::UnsupportedPublication(_))));
    assert_eq!(std::fs::read(temp.dest()).unwrap(), b"prior complete");
    assert!(!transaction.unwrap().exists());
}
