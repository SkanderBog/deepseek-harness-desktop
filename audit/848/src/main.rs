#[allow(dead_code)]
#[path = "../../../src-tauri/src/service/backup/archive.rs"]
mod archive;
use std::{fs, path::PathBuf};

fn main() {
    let root = std::env::temp_dir().join(format!("dsh-848-audit-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    fs::create_dir(&root).unwrap();
    let result = std::panic::catch_unwind(|| probe(&root));
    fs::remove_dir_all(&root).unwrap();
    if let Err(error) = result { std::panic::resume_unwind(error) }
}

fn probe(root: &std::path::Path) {
    let saved = root.join("saved-sessions");
    let relative = PathBuf::from("project/session-a/session.v4.jsonl.zstd");
    fs::create_dir_all(saved.join(relative.parent().unwrap())).unwrap();
    fs::write(saved.join(&relative), b"turn 10").unwrap();
    let archive_path = root.join("backup.tar.zst");
    archive::create_archive_sections(&[(archive::SESSIONS_SECTION, &saved)], &archive_path, false).unwrap();
    let live = root.join("live-sessions");
    fs::create_dir_all(live.join(relative.parent().unwrap())).unwrap();
    fs::write(live.join(&relative), b"turn 20").unwrap();
    let found = archive::extract_archive_section(&archive_path, archive::SESSIONS_SECTION, &live).unwrap();
    assert!(found);
    let after = fs::read_to_string(live.join(&relative)).unwrap();
    println!("AUDIT session_merge_before=turn20 after={after:?} preserved_newer={}", after == "turn 20");
    let profile = root.join("restored-profile");
    let profile_found = archive::extract_archive_section(&archive_path, archive::PROFILE_SECTION, &profile).unwrap();
    println!("AUDIT missing_profile_result={profile_found} profile_directory_created={}", profile.is_dir());

    let external = root.join("external");
    let redirected = root.join("redirected-sessions");
    fs::create_dir_all(external.join("session-a")).unwrap();
    fs::create_dir(&redirected).unwrap();
    fs::write(external.join("session-a/session.v4.jsonl.zstd"), b"outside sentinel").unwrap();
    let link = redirected.join("project");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&external, &link).unwrap();
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&link).arg(&external).status().unwrap();
        assert!(status.success());
    }
    let restored = archive::extract_archive_section(&archive_path, archive::SESSIONS_SECTION, &redirected);
    let after = fs::read_to_string(external.join("session-a/session.v4.jsonl.zstd")).unwrap();
    println!("AUDIT redirected_restore_ok={} external_preserved={} after={after:?}", restored.is_ok(), after == "outside sentinel");
    #[cfg(windows)]
    fs::remove_dir(link).unwrap();
}
