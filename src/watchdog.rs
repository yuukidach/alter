//! A separate process releases the compositor's keyboard grab if GTK stalls.

use gtk::{glib, prelude::*};
use rustix::process::{PidfdFlags, Signal, getppid, pidfd_open, pidfd_send_signal};
use std::io::{self, Read, Write};
use std::os::unix::{net::UnixStream, process::CommandExt};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

pub const ARGUMENT: &str = "--internal-focus-watchdog";
const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(250);
const STALL_TIMEOUT: Duration = Duration::from_secs(8);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(3);
const READY: u8 = b'R';
const VISIBLE: u8 = b'V';
const HIDDEN: u8 = b'H';

pub struct Watchdog {
    process: Rc<MonitorProcess>,
    timer: Option<glib::SourceId>,
}

impl Watchdog {
    pub fn start(application: &gtk::Application) -> io::Result<Self> {
        let mut command = Command::new(std::env::current_exe()?);
        command.arg(ARGUMENT);
        let process = Rc::new(MonitorProcess::spawn(command)?);
        let heartbeat = process.clone();
        let application = application.downgrade();
        let timer = glib::timeout_add_local(HEARTBEAT_INTERVAL, move || {
            if let Some(application) = application.upgrade() {
                // Only GTK's main thread may send heartbeats. A worker could
                // keep a stuck main loop looking healthy indefinitely.
                let visible = application
                    .windows()
                    .iter()
                    .any(|window| window.is_visible());
                heartbeat.send_or_exit(visible);
            }
            glib::ControlFlow::Continue
        });
        Ok(Self {
            process,
            timer: Some(timer),
        })
    }

    pub fn arm(&self) {
        // Arm BEFORE mapping, not on the first timer tick after mapping: GTK
        // can hang while presenting a surface that already owns the keyboard.
        self.process.send_or_exit(true);
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
    }
}

struct MonitorProcess {
    socket: UnixStream,
    child: Child,
}

impl MonitorProcess {
    fn spawn(mut command: Command) -> io::Result<Self> {
        let (socket, peer) = UnixStream::pair()?;
        // A process-group stop of the launcher must not stop its watchdog too.
        let child = command
            .process_group(0)
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(peer)))
            .stdout(Stdio::null())
            .spawn()?;
        drop(command);
        let mut process = Self { socket, child };
        process.socket.set_read_timeout(Some(STARTUP_TIMEOUT))?;
        let mut ready = [0];
        process.socket.read_exact(&mut ready)?;
        if ready != [READY] {
            return Err(io::Error::other("invalid focus watchdog handshake"));
        }
        process.socket.set_read_timeout(None)?;
        process.socket.set_nonblocking(true)?;
        Ok(process)
    }

    fn send(&self, visible: bool) -> io::Result<()> {
        // Never let a stalled watchdog block GTK on a full IPC buffer.
        (&self.socket).write_all(&[if visible { VISIBLE } else { HIDDEN }])
    }

    fn send_or_exit(&self, visible: bool) {
        if let Err(error) = self.send(visible) {
            eprintln!("alter: focus watchdog unavailable; exiting safely: {error}");
            std::process::exit(1);
        }
    }
}

impl Drop for MonitorProcess {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
struct Heartbeat {
    last_visible: Option<Instant>,
}

impl Heartbeat {
    fn receive(&mut self, message: u8, now: Instant) -> io::Result<()> {
        self.last_visible = match message {
            VISIBLE => Some(now),
            HIDDEN => None,
            _ => return Err(io::Error::other("invalid focus watchdog heartbeat")),
        };
        Ok(())
    }

    fn expired(&self, now: Instant, timeout: Duration) -> bool {
        self.last_visible
            .is_some_and(|last| now.duration_since(last) >= timeout)
    }
}

pub fn run() -> io::Result<()> {
    run_with_timeout(STALL_TIMEOUT)
}

fn run_with_timeout(timeout: Duration) -> io::Result<()> {
    // This private mode accepts only an inherited socket, never an arbitrary
    // target PID. Pin our parent with a pidfd so PID reuse cannot kill a new app.
    let mut socket = UnixStream::from(rustix::io::dup(io::stdin())?);
    socket.set_read_timeout(Some(HEARTBEAT_INTERVAL))?;
    let parent = getppid()
        .filter(|pid| pid.as_raw_nonzero().get() > 1)
        .ok_or_else(|| io::Error::other("focus watchdog has no launcher parent"))?;
    let parent_fd = pidfd_open(parent, PidfdFlags::empty())?;
    if getppid() != Some(parent) {
        return Ok(());
    }
    socket.write_all(&[READY])?;

    let mut heartbeat = Heartbeat::default();
    let mut messages = [0; 64];
    loop {
        match socket.read(&mut messages) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                for &message in &messages[..count] {
                    heartbeat.receive(message, Instant::now())?;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
        if heartbeat.expired(Instant::now(), timeout) {
            pidfd_send_signal(&parent_fd, Signal::KILL)?;
            eprintln!("alter: GTK stopped responding while visible; released keyboard focus");
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn hidden_daemon_does_not_expire() {
        let now = Instant::now();
        let mut heartbeat = Heartbeat::default();
        assert!(!heartbeat.expired(now + STALL_TIMEOUT, STALL_TIMEOUT));
        heartbeat.receive(VISIBLE, now).unwrap();
        heartbeat.receive(HIDDEN, now).unwrap();
        assert!(!heartbeat.expired(now + STALL_TIMEOUT * 100, STALL_TIMEOUT));
    }

    #[test]
    fn visible_heartbeat_expires_at_timeout_and_refreshes() {
        let now = Instant::now();
        let mut heartbeat = Heartbeat::default();
        heartbeat.receive(VISIBLE, now).unwrap();
        assert!(!heartbeat.expired(now + STALL_TIMEOUT - HEARTBEAT_INTERVAL, STALL_TIMEOUT));
        assert!(heartbeat.expired(now + STALL_TIMEOUT, STALL_TIMEOUT));
        heartbeat.receive(VISIBLE, now + STALL_TIMEOUT).unwrap();
        assert!(!heartbeat.expired(now + STALL_TIMEOUT, STALL_TIMEOUT));
    }

    #[test]
    fn invalid_messages_do_not_disarm_the_watchdog() {
        let now = Instant::now();
        let mut heartbeat = Heartbeat::default();
        heartbeat.receive(VISIBLE, now).unwrap();
        assert!(heartbeat.receive(b'?', now).is_err());
        assert!(heartbeat.expired(now + STALL_TIMEOUT, STALL_TIMEOUT));
    }

    fn helper(role: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "watchdog::tests::subprocess", "--ignored"])
            .env("ALTER_WATCHDOG_TEST_ROLE", role);
        command
    }

    #[test]
    #[ignore = "only invoked as an isolated watchdog test subprocess"]
    fn subprocess() {
        match std::env::var("ALTER_WATCHDOG_TEST_ROLE").as_deref() {
            Ok("monitor") => run_with_timeout(Duration::from_millis(600)).unwrap(),
            Ok("stopped-launcher") => {
                let monitor = MonitorProcess::spawn(helper("monitor")).unwrap();
                monitor.send(true).unwrap();
                // No GTK display is needed to simulate the entire launcher
                // freezing, including any in-process watchdog threads.
                rustix::process::kill_process_group(rustix::process::getpid(), Signal::STOP)
                    .unwrap();
                panic!("stopped launcher unexpectedly resumed");
            }
            Ok("responsive-launcher") => {
                let mut monitor = MonitorProcess::spawn(helper("monitor")).unwrap();
                for _ in 0..12 {
                    monitor.send(true).unwrap();
                    std::thread::sleep(Duration::from_millis(80));
                }
                monitor.send(false).unwrap();
                std::thread::sleep(Duration::from_millis(900));
                assert!(monitor.child.try_wait().unwrap().is_none());
                monitor.socket.shutdown(std::net::Shutdown::Both).unwrap();
                assert!(wait_for_exit(&mut monitor.child).success());
            }
            _ => {}
        }
    }

    fn wait_for_exit(child: &mut Child) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("watchdog test subprocess failed to exit");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn separate_watchdog_kills_a_stopped_launcher() {
        let mut child = helper("stopped-launcher")
            .process_group(0)
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        assert_eq!(wait_for_exit(&mut child).signal(), Some(9));
    }

    #[test]
    fn live_and_hidden_launcher_survives_and_monitor_exits_on_disconnect() {
        let mut child = helper("responsive-launcher")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        assert!(wait_for_exit(&mut child).success());
    }

    #[test]
    fn startup_rejects_a_child_without_a_handshake() {
        let command = Command::new("true");
        assert!(MonitorProcess::spawn(command).is_err());
    }

    #[test]
    fn disconnect_is_detected_without_blocking_gtk() {
        let mut monitor = MonitorProcess::spawn(helper("monitor")).unwrap();
        monitor.child.kill().unwrap();
        monitor.child.wait().unwrap();
        assert!(monitor.send(true).is_err());
    }

    #[test]
    fn drop_reaps_the_monitor() {
        let monitor = MonitorProcess::spawn(helper("monitor")).unwrap();
        let pid = rustix::process::Pid::from_child(&monitor.child);
        drop(monitor);
        assert!(matches!(
            rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG),
            Err(rustix::io::Errno::CHILD)
        ));
    }
}
