//! The process's own figures, under the names every Prometheus client gives
//! them (`process_*`), so a dashboard made for any other server reads this one.
//!
//! Linux only, read from `/proc`: that is where the server runs - the image and
//! the release archives - and the one place these figures are a file away. On
//! any other system the scrape carries the start time alone, rather than
//! figures assembled from a second source per platform.

/// What `/proc` says about this process at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Process {
    /// User and system time together, in seconds.
    pub cpu_seconds: f64,
    pub resident_bytes: u64,
    pub virtual_bytes: u64,
    pub threads: u64,
    pub open_fds: u64,
    /// The soft limit, which is the one an `open` fails against. `None` when
    /// the limit is `unlimited`.
    pub max_fds: Option<u64>,
}

/// Reads the figures, or `None` where there is no `/proc` to read them from.
///
/// A file that cannot be read or parsed is `None` too, and logged: a scrape
/// that carries no process figures is a gap on a graph, and one that fails
/// over them would take the server's other figures down with them.
#[cfg(target_os = "linux")]
pub fn read() -> Option<Process> {
    let read = || -> Result<Process, String> {
        let file = |path: &str| std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"));
        let ticks = clock_ticks_per_second().ok_or("the clock tick rate is unknown")?;
        let cpu_ticks = cpu_ticks(&file("/proc/self/stat")?).ok_or("/proc/self/stat has no CPU times")?;
        let status = file("/proc/self/status")?;
        let open_fds = std::fs::read_dir("/proc/self/fd").map_err(|error| format!("/proc/self/fd: {error}"))?.count();
        Ok(Process {
            cpu_seconds: cpu_ticks as f64 / ticks as f64,
            resident_bytes: status_kibibytes(&status, "VmRSS").ok_or("/proc/self/status has no VmRSS")? * 1024,
            virtual_bytes: status_kibibytes(&status, "VmSize").ok_or("/proc/self/status has no VmSize")? * 1024,
            threads: status_count(&status, "Threads").ok_or("/proc/self/status has no Threads")?,
            open_fds: open_fds as u64,
            max_fds: max_open_files(&file("/proc/self/limits")?).ok_or("/proc/self/limits has no open files limit")?,
        })
    };
    read().inspect_err(|error| tracing::warn!(%error, "failed to read the process figures")).ok()
}

#[cfg(not(target_os = "linux"))]
pub fn read() -> Option<Process> {
    None
}

/// How many clock ticks `/proc/self/stat` counts in a second.
#[cfg(target_os = "linux")]
fn clock_ticks_per_second() -> Option<u64> {
    // SAFETY: `sysconf` reads a constant of the running system; it takes no
    // pointers and has no preconditions.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    u64::try_from(ticks).ok().filter(|ticks| *ticks > 0)
}

/// User plus system time, in clock ticks, from `/proc/self/stat`.
///
/// The second field is the command name in parentheses, and a name may hold
/// spaces and parentheses of its own - so the fields are counted from the last
/// `)`, where `utime` and `stime` are the 12th and 13th.
#[cfg(any(target_os = "linux", test))]
fn cpu_ticks(stat: &str) -> Option<u64> {
    let (_, after_name) = stat.rsplit_once(')')?;
    let mut fields = after_name.split_whitespace().skip(11);
    let user: u64 = fields.next()?.parse().ok()?;
    let system: u64 = fields.next()?.parse().ok()?;
    Some(user + system)
}

/// A `Key:   1234 kB` line of `/proc/self/status`, in kibibytes.
#[cfg(any(target_os = "linux", test))]
fn status_kibibytes(status: &str, key: &str) -> Option<u64> {
    let value = status_value(status, key)?;
    value.strip_suffix("kB")?.trim().parse().ok()
}

/// A `Key:   12` line of `/proc/self/status`.
#[cfg(any(target_os = "linux", test))]
fn status_count(status: &str, key: &str) -> Option<u64> {
    status_value(status, key)?.parse().ok()
}

#[cfg(any(target_os = "linux", test))]
fn status_value<'a>(status: &'a str, key: &str) -> Option<&'a str> {
    status.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name == key).then(|| value.trim())
    })
}

/// The soft limit on open files, from `/proc/self/limits`: `Some(None)` for
/// `unlimited`, `None` when the line is not there to read.
#[cfg(any(target_os = "linux", test))]
fn max_open_files(limits: &str) -> Option<Option<u64>> {
    let line = limits.lines().find(|line| line.starts_with("Max open files"))?;
    let soft = line.trim_start_matches("Max open files").split_whitespace().next()?;
    match soft {
        "unlimited" => Some(None),
        number => number.parse().ok().map(Some),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cpu_times_are_counted_from_the_last_parenthesis() {
        // A command name with a space and a `)` of its own: counting fields
        // from the start would read the name's halves as fields.
        let stat = "4242 (kasl (server) 1) S 1 4242 4242 0 -1 4194560 2861 0 0 0 750 125 0 0 20 0 9 0 3518 98304000 4120 18446744073709551615";
        assert_eq!(cpu_ticks(stat), Some(875));
        assert_eq!(cpu_ticks("4242 kasl-server S 1"), None, "no name in parentheses is no stat line");
    }

    #[test]
    fn the_status_lines_are_read_by_their_key() {
        let status = "Name:\tkasl-server\nVmPeak:\t  120000 kB\nVmSize:\t   98304 kB\nVmRSS:\t   16480 kB\nThreads:\t9\n";
        assert_eq!(status_kibibytes(status, "VmRSS"), Some(16_480));
        assert_eq!(status_kibibytes(status, "VmSize"), Some(98_304));
        assert_eq!(status_count(status, "Threads"), Some(9));
        assert_eq!(status_kibibytes(status, "VmSwap"), None);
        // `VmSize` is not read off `VmPeak` or any other line it is a prefix of.
        assert_eq!(status_kibibytes("VmSizeX:\t1 kB\n", "VmSize"), None);
    }

    #[test]
    fn the_open_files_limit_is_the_soft_one() {
        let limits = "Limit                     Soft Limit           Hard Limit           Units     \n\
                      Max processes             63340                63340                processes \n\
                      Max open files            1024                 524288               files     \n";
        assert_eq!(max_open_files(limits), Some(Some(1024)));

        let unlimited = "Max open files            unlimited            unlimited            files     \n";
        assert_eq!(max_open_files(unlimited), Some(None));
        assert_eq!(max_open_files("Max processes 1 1 processes\n"), None);
    }
}
