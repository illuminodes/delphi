use std::fmt::Write as _;
use std::time::Duration;

use crate::{LogWatch, ProcSample, SampleSeries};

/// What a soak run measured, sample by sample, and its closing summary.
#[derive(Debug, Clone, Default)]
pub struct SoakReport {
    rss_kib: SampleSeries,
    disk_bytes: SampleSeries,
    threads: SampleSeries,
    open_files: SampleSeries,
    kernel_peak_rss_kib: u64,
    first: Option<ProcSample>,
    last: ProcSample,
    elapsed: Duration,
}

impl SoakReport {
    /// Record one reading taken `elapsed` into the run.
    pub fn add(&mut self, sample: ProcSample, disk_bytes: u64, elapsed: Duration) {
        self.rss_kib.add(sample.rss_kib);
        self.disk_bytes.add(disk_bytes);
        self.threads.add(sample.threads);
        self.open_files.add(sample.open_files);
        self.kernel_peak_rss_kib = self.kernel_peak_rss_kib.max(sample.peak_rss_kib);
        self.first.get_or_insert(sample);
        self.last = sample;
        self.elapsed = elapsed;
    }

    #[must_use]
    pub const fn samples(&self) -> u64 {
        self.rss_kib.count()
    }

    /// One progress line for the latest reading.
    #[must_use]
    pub fn line(&self, disk_bytes: u64, ticks_per_second: u64, watch: &LogWatch) -> String {
        format!(
            "[{:>6}s] rss {} MiB (peak {} MiB) | disk {} MiB | cpu {:.0}% | threads {} | fds {} | io r {} MiB w {} MiB | passes {} failures {} published {} | {}",
            self.elapsed.as_secs(),
            self.last.rss_kib / 1024,
            self.kernel_peak_rss_kib / 1024,
            disk_bytes >> 20,
            self.cpu_percent(ticks_per_second),
            self.last.threads,
            self.last.open_files,
            self.last.read_bytes >> 20,
            self.last.write_bytes >> 20,
            watch.chain_passes,
            watch.sync_failures,
            watch.published,
            watch
                .last_chain
                .as_deref()
                .or(watch.last_headers.as_deref())
                .unwrap_or("no headers yet"),
        )
    }

    /// Mean CPU use over the run, as a percentage of one core.
    #[must_use]
    pub fn cpu_percent(&self, ticks_per_second: u64) -> f64 {
        let used = self.last.cpu_ticks.saturating_sub(self.first.map_or(0, |first| first.cpu_ticks));
        let seconds = self.elapsed.as_secs_f64();
        if seconds <= 0.0 || ticks_per_second == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let used = used as f64 / ticks_per_second as f64;
        100.0 * used / seconds
    }

    /// The closing summary.
    #[must_use]
    pub fn summary(&self, ticks_per_second: u64, watch: &LogWatch, exit: &str) -> String {
        let mut text = String::new();
        let mib = |kib: u64| kib / 1024;
        let _ = writeln!(text, "duration           {} s over {} samples", self.elapsed.as_secs(), self.samples());
        let _ = writeln!(text, "daemon exit        {exit}");
        let _ = writeln!(text, "resident peak      {} MiB (kernel high-water mark)", mib(self.kernel_peak_rss_kib));
        let _ = writeln!(text, "resident peak      {} MiB (sampled)", mib(self.rss_kib.peak()));
        let _ = writeln!(text, "resident mean      {} MiB", mib(self.rss_kib.mean()));
        let _ = writeln!(text, "disk peak          {} MiB", self.disk_bytes.peak() >> 20);
        let _ = writeln!(text, "disk mean          {} MiB", self.disk_bytes.mean() >> 20);
        let _ = writeln!(text, "cpu mean           {:.1}% of one core", self.cpu_percent(ticks_per_second));
        let _ = writeln!(text, "threads peak       {}", self.threads.peak());
        let _ = writeln!(text, "open files peak    {}", self.open_files.peak());
        let _ = writeln!(text, "disk read total    {} MiB", self.last.read_bytes >> 20);
        let _ = writeln!(text, "disk written total {} MiB", self.last.write_bytes >> 20);
        let _ = writeln!(text, "chain passes       {}", watch.chain_passes);
        let _ = writeln!(text, "chain failures     {}", watch.sync_failures);
        let _ = writeln!(text, "notes published    {}", watch.published);
        let _ = writeln!(text, "notes rejected     {}", watch.rejected);
        let _ = writeln!(text, "notes undelivered  {}", watch.undelivered);
        let _ = writeln!(text, "mempool reconnects {}", watch.mempool_reconnects);
        let _ = writeln!(text, "last chain pass    {}", watch.last_chain.as_deref().unwrap_or("none"));
        let _ = writeln!(text, "last header page   {}", watch.last_headers.as_deref().unwrap_or("none"));
        if let Some(fatal) = &watch.fatal {
            let _ = writeln!(text, "fatal              {fatal}");
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::SoakReport;
    use crate::{LogWatch, ProcSample};

    fn sample(rss_kib: u64, cpu_ticks: u64) -> ProcSample {
        ProcSample {
            rss_kib,
            peak_rss_kib: rss_kib + 1024,
            cpu_ticks,
            threads: 9,
            open_files: 40,
            read_bytes: 0,
            write_bytes: 3 << 20,
        }
    }

    #[test]
    fn the_summary_reports_peak_and_mean() {
        let mut report = SoakReport::default();
        report.add(sample(100 * 1024, 0), 10 << 20, Duration::from_secs(0));
        report.add(sample(300 * 1024, 500), 30 << 20, Duration::from_secs(10));
        let summary = report.summary(100, &LogWatch::default(), "exit status: 0");

        assert!(summary.contains("resident peak      301 MiB (kernel"), "{summary}");
        assert!(summary.contains("resident peak      300 MiB (sampled)"), "{summary}");
        assert!(summary.contains("resident mean      200 MiB"), "{summary}");
        assert!(summary.contains("disk peak          30 MiB"), "{summary}");
        assert!(summary.contains("disk mean          20 MiB"), "{summary}");
        assert!(summary.contains("cpu mean           50.0%"), "{summary}");
    }

    #[test]
    fn no_time_elapsed_is_no_cpu() {
        let mut report = SoakReport::default();
        report.add(sample(1, 50), 0, Duration::ZERO);

        assert!(report.cpu_percent(100).abs() < f64::EPSILON);
    }
}
