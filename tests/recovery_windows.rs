#![cfg(windows)]

use tessera::{error::AppError, recovery::RecoveryStore};

#[test]
fn recovery_refuses_unverified_directory_durability() {
    let path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-windows-recovery-{}", std::process::id()));
    assert!(!path.exists());
    // Creation and reopening must both fail before retaining any sensitive checkpoint.
    for _ in 0..2 {
        assert!(matches!(
            RecoveryStore::open_at(&path),
            Err(AppError::UnsupportedPublication(_))
        ));
    }
    for entry in std::fs::read_dir(&path).unwrap() {
        let session = entry.unwrap().path();
        assert!(session.is_dir());
        assert_eq!(std::fs::read_dir(session).unwrap().count(), 0);
    }
    std::fs::remove_dir_all(path).unwrap();
}
