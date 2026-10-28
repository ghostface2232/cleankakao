// Portable update handoff. Only the helper replaces files after both app processes exit.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_PARAMETER, HANDLE, LPARAM, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
use windows::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_OK, MessageBoxW, PostThreadMessageW, WM_QUIT,
};
use windows::core::HSTRING;

use crate::updater::{ReleaseInfo, UpdateChecker};

pub const HELPER_FLAG: &str = "--apply-update";
const FILES: &[&str] = &["helper.exe", "payload.exe", "previous.exe", "ready"];

pub async fn prepare(release: ReleaseInfo) -> Result<(), String> {
    let target = std::env::current_exe().map_err(|e| e.to_string())?;
    // Staging beside the target tests write access and keeps replacement on one volume.
    let staging = tempfile::Builder::new()
        .prefix(".cleankakao-update-")
        .tempdir_in(target.parent().ok_or("missing executable directory")?)
        .map_err(|e| e.to_string())?;
    UpdateChecker::current()
        .download(&release, &staging.path().join("payload.exe"))
        .await?;
    std::fs::copy(&target, staging.path().join("helper.exe")).map_err(|e| e.to_string())?;
    let args: Vec<String> = std::env::args().collect();
    let host_pid = argument(&args, "--host-pid").unwrap_or("0");
    let host_thread = argument(&args, "--host-thread").unwrap_or("0");
    let mut child = Command::new(staging.path().join("helper.exe"))
        .arg(HELPER_FLAG)
        .arg(&target)
        .arg(host_pid)
        .arg(host_thread)
        .arg(std::process::id().to_string())
        .arg(&release.sha256)
        .spawn()
        .map_err(|e| e.to_string())?;
    for _ in 0..100 {
        if staging.path().join("ready").exists() {
            // The helper and the restarted app now own the known staging files.
            let _ = staging.keep();
            return Ok(());
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("update helper could not start".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = child.kill();
    let _ = child.wait();
    Err("update helper timed out".into())
}

pub fn argument<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
}

struct Process(HANDLE);
impl Process {
    fn open(pid: u32) -> windows::core::Result<Self> {
        // SAFETY: Requests only synchronization access to the supplied process ID.
        unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }.map(Self)
    }

    fn open_host(pid: u32) -> Result<Option<Self>, String> {
        if pid == 0 {
            return Ok(None);
        }
        match Self::open(pid) {
            Ok(process) => Ok(Some(process)),
            // An absent PID is already stopped. Permission errors are not evidence
            // of exit and must still abort the update.
            Err(error) if error.code() == ERROR_INVALID_PARAMETER.to_hresult() => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn has_exited(&self) -> Result<bool, String> {
        // SAFETY: This guard owns a live synchronization handle; zero timeout
        // only probes its signaled state and never blocks the helper.
        match unsafe { WaitForSingleObject(self.0, 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(windows::core::Error::from_thread().to_string()),
        }
    }
    fn wait(&self) -> Result<(), String> {
        // SAFETY: This guard owns a live process handle for the entire wait.
        if unsafe { WaitForSingleObject(self.0, 30_000) } == WAIT_OBJECT_0 {
            Ok(())
        } else {
            Err("application did not exit within 30 seconds".into())
        }
    }
}

fn request_host_shutdown(
    host: Option<&Process>,
    post_quit: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let Some(host) = host else {
        return Ok(());
    };
    if host.has_exited()? {
        return Ok(());
    }
    if let Err(error) = post_quit() {
        // The tray may exit between the state probe and PostThreadMessageW.
        // Suppress the posting error only after confirming that it exited.
        if !host.has_exited()? {
            return Err(error);
        }
    }
    Ok(())
}
impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: This guard closes its owned handle exactly once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

pub fn run_helper() {
    if let Err(error) = apply() {
        log::error!("update failed: {error}");
        // SAFETY: Both HSTRING buffers live throughout this synchronous call.
        unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(format!("업데이트를 완료하지 못했습니다.\n{error}")),
                &HSTRING::from("CleanKakao 업데이트"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

fn apply() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 7 {
        return Err("invalid update arguments".into());
    }
    let target = PathBuf::from(&args[2]);
    let host_pid: u32 = args[3].parse().map_err(|_| "invalid host PID")?;
    let host_thread: u32 = args[4].parse().map_err(|_| "invalid host thread")?;
    let settings_pid: u32 = args[5].parse().map_err(|_| "invalid settings PID")?;
    let helper = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = helper.parent().ok_or("missing helper directory")?;
    if !valid_staging(dir, &target) {
        return Err("invalid staging directory".into());
    }
    let payload = dir.join("payload.exe");
    let bytes = std::fs::read(&payload).map_err(|e| e.to_string())?;
    if format!("{:x}", Sha256::digest(&bytes)) != args[6] || !bytes.starts_with(b"MZ") {
        return Err("invalid update executable or digest".into());
    }
    let settings = Process::open(settings_pid).map_err(|e| e.to_string())?;
    let host = Process::open_host(host_pid)?;
    request_host_shutdown(host.as_ref(), || {
        // SAFETY: The ID was supplied by our tray's settings launch. WM_QUIT requests
        // normal message-loop shutdown; it does not terminate an arbitrary process.
        unsafe { PostThreadMessageW(host_thread, WM_QUIT, WPARAM(0), LPARAM(0)) }
            .map_err(|e| e.to_string())
    })?;
    std::fs::write(dir.join("ready"), b"ready").map_err(|e| e.to_string())?;
    settings.wait()?;
    if let Some(host) = host {
        host.wait()?;
    }
    replace_and_restart(&target, &payload, dir, |path, updated| {
        let mut command = Command::new(path);
        if updated {
            command.arg("--updated").arg("--cleanup-update").arg(dir);
        }
        command.spawn().map(|_| ())
    })
}

fn replace_and_restart(
    target: &Path,
    payload: &Path,
    dir: &Path,
    mut launch: impl FnMut(&Path, bool) -> std::io::Result<()>,
) -> Result<(), String> {
    let backup = dir.join("previous.exe");
    // Rename can briefly fail while Windows/AV releases the old image mapping.
    let mut renamed = false;
    for _ in 0..100 {
        if std::fs::rename(target, &backup).is_ok() {
            renamed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !renamed {
        let _ = launch(target, false);
        return Err("could not replace the executable (check folder permissions)".into());
    }
    if let Err(error) = std::fs::rename(payload, target) {
        std::fs::rename(&backup, target)
            .map_err(|e| format!("restore failed: {e}; backup: {}", backup.display()))?;
        let _ = launch(target, false);
        return Err(error.to_string());
    }
    if let Err(error) = launch(target, true) {
        std::fs::rename(target, payload).map_err(|e| e.to_string())?;
        std::fs::rename(&backup, target).map_err(|e| e.to_string())?;
        let _ = launch(target, false);
        return Err(error.to_string());
    }
    Ok(())
}

fn valid_staging(dir: &Path, target: &Path) -> bool {
    target.is_absolute()
        && dir.is_absolute()
        && dir.parent() == target.parent()
        && dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".cleankakao-update-"))
}

pub fn cleanup(args: &[String]) {
    let Some(dir) = argument(args, "--cleanup-update").map(PathBuf::from) else {
        return;
    };
    let Ok(target) = std::env::current_exe() else {
        return;
    };
    if !valid_staging(&dir, &target) {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..100 {
            for name in FILES {
                let _ = std::fs::remove_file(dir.join(name));
            }
            if std::fs::remove_dir(&dir).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Stdio};

    #[test]
    fn wait_for_test_exit() {
        if std::env::var_os("CLEANKAKAO_UPDATE_TEST_CHILD").is_some() {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).unwrap();
        }
    }

    fn waiting_child() -> (Child, Process) {
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "update_install::tests::wait_for_test_exit"])
            .env("CLEANKAKAO_UPDATE_TEST_CHILD", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()
            .unwrap();
        let process = Process::open(child.id()).unwrap();
        (child, process)
    }

    #[test]
    fn skips_shutdown_for_absent_or_already_exited_host() {
        assert!(Process::open_host(0).unwrap().is_none());
        assert!(Process::open_host(u32::MAX).unwrap().is_none());
        request_host_shutdown(None, || panic!("no host to stop")).unwrap();
        let (mut child, host) = waiting_child();
        drop(child.stdin.take());
        assert!(child.wait().unwrap().success());
        assert!(host.has_exited().unwrap());
        request_host_shutdown(Some(&host), || panic!("host already exited")).unwrap();
    }

    #[test]
    fn tolerates_host_exit_during_shutdown_request() {
        let (mut child, host) = waiting_child();
        assert!(!host.has_exited().unwrap());
        request_host_shutdown(Some(&host), || {
            drop(child.stdin.take());
            assert!(child.wait().unwrap().success());
            Err("thread has exited".into())
        })
        .unwrap();
    }

    #[test]
    fn propagates_shutdown_errors_while_host_is_alive() {
        let host = Process::open(std::process::id()).unwrap();
        let result = request_host_shutdown(Some(&host), || Err("posting failed".into()));
        assert_eq!(result.unwrap_err(), "posting failed");
    }

    #[test]
    fn replaces_and_restarts_from_paths_with_spaces_and_unicode() {
        let root = tempfile::Builder::new()
            .prefix("업데이트 test ")
            .tempdir()
            .unwrap();
        let target = root.path().join("Clean Kakao.exe");
        let stage = root.path().join(".cleankakao-update-test");
        std::fs::create_dir(&stage).unwrap();
        let payload = stage.join("payload.exe");
        std::fs::write(&target, b"old").unwrap();
        std::fs::write(&payload, b"new").unwrap();
        let mut launches = 0;
        replace_and_restart(&target, &payload, &stage, |path, updated| {
            assert!(updated);
            assert_eq!(path, target);
            assert_eq!(std::fs::read(path).unwrap(), b"new");
            launches += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(launches, 1);
        assert_eq!(std::fs::read(stage.join("previous.exe")).unwrap(), b"old");
    }

    #[test]
    fn restores_original_when_payload_move_fails() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("app.exe");
        std::fs::write(&target, b"old").unwrap();
        let mut relaunched = false;
        let result = replace_and_restart(
            &target,
            &root.path().join("missing.exe"),
            root.path(),
            |path, updated| {
                assert!(!updated);
                assert_eq!(std::fs::read(path).unwrap(), b"old");
                relaunched = true;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(relaunched);
    }

    #[test]
    fn restores_and_relaunches_original_when_new_executable_cannot_launch() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("app.exe");
        let payload = root.path().join("payload.exe");
        std::fs::write(&target, b"old").unwrap();
        std::fs::write(&payload, b"new").unwrap();
        let mut launches = Vec::new();
        let result = replace_and_restart(&target, &payload, root.path(), |path, updated| {
            launches.push(updated);
            if updated {
                Err(std::io::Error::other("launch failed"))
            } else {
                assert_eq!(std::fs::read(path).unwrap(), b"old");
                Ok(())
            }
        });
        assert!(result.is_err());
        assert_eq!(launches, [true, false]);
        assert_eq!(std::fs::read(target).unwrap(), b"old");
    }

    #[test]
    fn cleanup_is_restricted_to_sibling_staging_folders() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("app.exe");
        assert!(valid_staging(
            &root.path().join(".cleankakao-update-abc"),
            &target
        ));
        assert!(!valid_staging(root.path(), &target));
        assert!(!valid_staging(&root.path().join("other"), &target));
        assert!(!valid_staging(
            &root.path().join("nested/.cleankakao-update-abc"),
            &target
        ));
    }
}
