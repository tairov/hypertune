mod wall_clock_timer;

#[cfg(windows)]
mod windows_timer;

#[cfg(not(windows))]
mod unix_timer;

#[cfg(target_os = "linux")]
use nix::fcntl::{splice, SpliceFFlags};
#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::os::unix::io::AsRawFd;

#[cfg(target_os = "windows")]
use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

use crate::util::units::Second;
use wall_clock_timer::WallClockTimer;

use std::io::Read;
use std::process::{ChildStdout, Command, ExitStatus};

use anyhow::Result;
use crate::options::CommandOutputPolicy;
use wait4::Wait4;
use crate::benchmark::custom_metric::MemUsageMetric;

#[cfg(not(windows))]
#[derive(Debug, Copy, Clone)]
struct CPUTimes {
    /// Total amount of time spent executing in user mode
    pub user_usec: i64,

    /// Total amount of time spent executing in kernel mode
    pub system_usec: i64,
}

/// Used to indicate the result of running a command
#[derive(Debug, Copy, Clone)]
pub struct TimerResult {
    pub time_real: Second,
    pub time_user: Second,
    pub time_system: Second,

    /// The exit status of the process
    pub status: ExitStatus,
    pub custom_metric: f64,
    pub mem_usage: MemUsageMetric,
}

/// Discard the output of a child process.
fn discard(output: ChildStdout) {
    const CHUNK_SIZE: usize = 64 << 10;

    #[cfg(target_os = "linux")]
    {
        if let Ok(file) = File::create("/dev/null") {
            while let Ok(bytes) = splice(
                output.as_raw_fd(),
                None,
                file.as_raw_fd(),
                None,
                CHUNK_SIZE,
                SpliceFFlags::empty(),
            ) {
                if bytes == 0 {
                    break;
                }
            }
        }
    }

    let mut output = output;
    let mut buf = [0; CHUNK_SIZE];
    while let Ok(bytes) = output.read(&mut buf) {
        if bytes == 0 {
            break;
        }
    }
}

/// Execute the given command and return a timing summary
pub fn execute_and_measure(mut command: Command, output_policy: &CommandOutputPolicy, collect_mem_usage: bool) -> Result<TimerResult> {
    #[cfg(not(windows))]
        let cpu_timer = self::unix_timer::CPUTimer::start();

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        // Create the process in a suspended state so that we don't miss any cpu time between process creation and `CPUTimer` start.
        command.creation_flags(CREATE_SUSPENDED);
    }

    let wallclock_timer = WallClockTimer::start();
    let mut child = command.spawn()?;

    #[cfg(windows)]
        let cpu_timer = {
        // SAFETY: We created a suspended process
        unsafe { self::windows_timer::CPUTimer::start_suspended_process(&child) }
    };

    let mut custom_metric: f64 = 0.0;

    if let Some(mut output) = child.stdout.take() {
        // Handle CommandOutputPolicy::Pipe
        if output_policy == &CommandOutputPolicy::Report {
            let mut s_metric = String::new();
            output.read_to_string(&mut s_metric).expect("Can't read from stdout");

            s_metric = s_metric.trim().to_string();
            custom_metric = match s_metric.trim().parse() {
                Ok(v) => v,
                Err(_) => 0.0 // or whatever error handling
            };
        }
        discard(output);
    }

    let status: ExitStatus;
    let mut mem_usage: MemUsageMetric = None;

    if collect_mem_usage {
        let res = child.wait4()?;
        status = res.status;
        mem_usage = Some(res.rusage.maxrss);
    } else {
        status = child.wait()?;
    }

    let time_real = wallclock_timer.stop();
    let (time_user, time_system) = cpu_timer.stop();
    Ok(TimerResult {
        time_real,
        time_user,
        time_system,
        status,
        custom_metric,
        mem_usage
    })
}
