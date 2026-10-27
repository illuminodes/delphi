use std::fs;
use std::io;

/// One reading of a running process's resource use, taken from `/proc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProcSample {
    pub rss_kib: u64,
    pub peak_rss_kib: u64,
    pub cpu_ticks: u64,
    pub threads: u64,
    pub open_files: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
}

impl ProcSample {
    /// # Errors
    /// Returns the I/O error if the process has gone or its status cannot be read.
    pub fn read(pid: u32) -> io::Result<Self> {
        let dir = format!("/proc/{pid}");
        let status = fs::read_to_string(format!("{dir}/status"))?;
        let stat = fs::read_to_string(format!("{dir}/stat"))?;
        let io = fs::read_to_string(format!("{dir}/io")).unwrap_or_default();
        let open_files = fs::read_dir(format!("{dir}/fd"))
            .map_or(0, |entries| u64::try_from(entries.count()).unwrap_or(u64::MAX));
        Ok(Self {
            open_files,
            ..Self::parse(&status, &stat, &io)
        })
    }

    /// The fields of `/proc/<pid>/status`, `stat` and `io`; missing ones read as zero.
    #[must_use]
    pub fn parse(status: &str, stat: &str, io: &str) -> Self {
        Self {
            rss_kib: Self::field(status, "VmRSS:"),
            peak_rss_kib: Self::field(status, "VmHWM:"),
            cpu_ticks: Self::cpu_ticks(stat),
            threads: Self::field(status, "Threads:"),
            open_files: 0,
            read_bytes: Self::field(io, "read_bytes:"),
            write_bytes: Self::field(io, "write_bytes:"),
        }
    }

    fn field(text: &str, key: &str) -> u64 {
        text.lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    }

    /// User plus system time. They are fields 14 and 15 of `stat`, counted
    /// after the parenthesised command name, which may itself hold spaces;
    /// the text after `)` starts at field 3.
    fn cpu_ticks(stat: &str) -> u64 {
        let Some((_, rest)) = stat.rsplit_once(')') else {
            return 0;
        };
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let tick = |index: usize| fields.get(index).and_then(|value| value.parse::<u64>().ok()).unwrap_or(0);
        tick(11) + tick(12)
    }
}

#[cfg(test)]
mod tests {
    use super::ProcSample;

    const STATUS: &str = "Name:\tdelphi-oracled\nVmHWM:\t  204800 kB\nVmRSS:\t  102400 kB\nThreads:\t9\n";
    const STAT: &str = "4242 (delphi (or) d) S 1 4242 4242 0 -1 4194560 100 0 0 0 1500 250 0 0 20 0 9 0";
    const IO: &str = "rchar: 10\nwchar: 20\nread_bytes: 4096\nwrite_bytes: 8192\ncancelled_write_bytes: 1\n";

    #[test]
    fn every_field_is_read() {
        let sample = ProcSample::parse(STATUS, STAT, IO);

        assert_eq!(sample.rss_kib, 102_400);
        assert_eq!(sample.peak_rss_kib, 204_800);
        assert_eq!(sample.threads, 9);
        assert_eq!(sample.cpu_ticks, 1_750);
        assert_eq!(sample.read_bytes, 4_096);
        assert_eq!(sample.write_bytes, 8_192);
    }

    #[test]
    fn missing_text_reads_as_zero() {
        assert_eq!(ProcSample::parse("", "", ""), ProcSample::default());
    }

    #[test]
    fn this_process_can_be_read() {
        let sample = ProcSample::read(std::process::id()).expect("own /proc entry");

        assert!(sample.rss_kib > 0);
        assert!(sample.threads >= 1);
        assert!(sample.open_files >= 1);
    }
}
