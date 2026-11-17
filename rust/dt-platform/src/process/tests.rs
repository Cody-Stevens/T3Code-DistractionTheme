use super::*;
use std::{
    fs,
    io::{BufRead, BufReader},
    process::Stdio,
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = kill_process_tree(&mut self.0);
    }
}

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("dt-platform-{}-{stamp}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn unique_mutex_rejects_second_instance_and_releases_on_drop() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("Local\\DtPlatformTest-{}-{stamp}", std::process::id());
    let guard = single_instance(&name).unwrap().unwrap();
    assert!(single_instance(&name).unwrap().is_none());
    drop(guard);
    assert!(single_instance(&name).unwrap().is_some());
    assert!(single_instance("").is_err());
    assert!(single_instance("bad\0name").is_err());
}

#[test]
fn liveness_distinguishes_current_missing_and_zero() {
    assert!(pid_alive(std::process::id()));
    assert!(!pid_alive(0));
    assert!(!pid_alive(u32::MAX));
}

#[test]
fn owner_acl_replaces_inheritance_for_directory_file_and_new_children() {
    let temp = TempDir::new();
    restrict_to_owner(&temp.0).unwrap();
    assert!(crate::win32::verify_owner_acl(&temp.0, true).unwrap());
    let file = temp.0.join("fake-token");
    fs::write(&file, b"not-a-real-token").unwrap();
    assert!(crate::win32::verify_owner_acl(&file, false).unwrap());
    restrict_to_owner(&file).unwrap();
    restrict_to_owner(&file).unwrap();
    assert!(crate::win32::verify_owner_acl(&file, true).unwrap());
    let sub = temp.0.join("nested");
    fs::create_dir(&sub).unwrap();
    let nested = sub.join("inherited");
    fs::write(&nested, b"fake").unwrap();
    assert!(crate::win32::verify_owner_acl(&nested, false).unwrap());
    assert!(restrict_to_owner(&temp.0.join("missing")).is_err());
}

// Test-only process fixture: no shells, ports, clipboard, real hotkeys or user files.
#[test]
#[ignore = "subprocess fixture, invoked by process-tree tests"]
// The exit role deliberately leaves a descendant to verify Job Object cleanup on root exit.
#[allow(clippy::zombie_processes)]
fn process_fixture() {
    let Ok(role) = std::env::var("DT_PLATFORM_FIXTURE_ROLE") else {
        return;
    };
    if role == "leaf" {
        thread::sleep(Duration::from_secs(30));
        return;
    }
    use std::os::windows::process::CommandExt;
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "process::tests::process_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("DT_PLATFORM_FIXTURE_ROLE", "leaf")
        .creation_flags(crate::win32::HIDDEN)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = if role == "owner-exit" {
        spawn_hidden(&mut command).unwrap()
    } else {
        command.spawn().unwrap()
    };
    println!("DT_GRANDCHILD {}", child.id());
    if role == "exit" || role == "owner-exit" {
        thread::sleep(Duration::from_millis(300));
    } else {
        child.wait().unwrap();
    }
}

fn tree(role: &str) -> (OwnedChild, u32, thread::JoinHandle<()>) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "process::tests::process_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("DT_PLATFORM_FIXTURE_ROLE", role)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    use std::os::windows::process::CommandExt;
    let mut child = OwnedChild(if role == "owner-exit" || role == "fallback" {
        command
            .creation_flags(crate::win32::HIDDEN)
            .spawn()
            .unwrap()
    } else {
        spawn_hidden(&mut command).unwrap()
    });
    let output = child.0.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(output)
            .lines()
            .map_while(std::result::Result::ok)
        {
            if let Some(pid) = line.strip_prefix("DT_GRANDCHILD ") {
                let _ = sender.send(pid.parse::<u32>().unwrap());
            }
        }
    });
    let descendant = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    (child, descendant, reader)
}

fn wait_dead(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while pid_alive(pid) {
        assert!(
            Instant::now() < deadline,
            "process {pid} survived job shutdown"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn job_shutdown_kills_child_and_grandchild_and_reaps_root() {
    let (mut child, grandchild, reader) = tree("wait");
    let root = child.0.id();
    assert!(pid_alive(root));
    assert!(pid_alive(grandchild));
    kill_process_tree(&mut child.0).unwrap();
    assert!(!pid_alive(root));
    assert!(!pid_alive(grandchild));
    assert!(child.0.try_wait().unwrap().is_some());
    kill_process_tree(&mut child.0).unwrap();
    reader.join().unwrap();
    println!("job shutdown: child and grandchild stopped and root reaped");
}

#[test]
fn normal_root_exit_also_stops_descendants() {
    let (mut child, grandchild, reader) = tree("exit");
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    wait_dead(grandchild);
    kill_process_tree(&mut child.0).unwrap();
    reader.join().unwrap();
}

#[test]
fn missing_executable_fails_without_a_child() {
    let mut command = Command::new("dt-platform-missing-executable-49ec7.exe");
    assert!(spawn_hidden(&mut command).is_err());
}

#[test]
fn taskkill_fallback_stops_an_owned_unmanaged_tree() {
    let (mut child, grandchild, reader) = tree("fallback");
    let root = child.0.id();
    kill_process_tree(&mut child.0).unwrap();
    assert!(!pid_alive(root));
    assert!(!pid_alive(grandchild));
    reader.join().unwrap();
}

#[test]
fn closing_job_owner_process_stops_its_child_without_a_monitor() {
    // This owner is outside spawn_hidden, so only its own job can kill its descendant.
    let (mut owner, descendant, reader) = tree("owner-exit");
    let deadline = Instant::now() + Duration::from_secs(5);
    while owner.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let stopped = Instant::now() + Duration::from_secs(5);
    while pid_alive(descendant) && Instant::now() < stopped {
        thread::sleep(Duration::from_millis(5));
    }
    if pid_alive(descendant) {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill.exe")
            .args(["/T", "/F", "/PID", &descendant.to_string()])
            .creation_flags(crate::win32::HIDDEN)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        panic!("descendant survived job owner exit");
    }
    reader.join().unwrap();
}
