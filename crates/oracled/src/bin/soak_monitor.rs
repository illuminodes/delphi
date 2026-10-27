use std::path::PathBuf;
use std::process::{Child, Command, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use delphi_oracled::{DirUsage, LogWatch, ProcSample, SoakReport};

/// Runs `delphi-oracled` for a fixed time and reports its resource use.
///
/// Usage: `soak_monitor DAEMON DATA_DIR SECONDS [SAMPLE_SECS [REPORT_SECS]] [-- DAEMON_ARGS...]`
/// The daemon gets `--data-dir`, `--log-file` and `--run-seconds` from here;
/// anything after `--` is passed on as well. The summary goes to stdout and
/// to `soak-summary.txt` beside the data directory.
struct SoakApp {
    daemon: PathBuf,
    data_dir: PathBuf,
    seconds: u64,
    sample: Duration,
    report: Duration,
    extra: Vec<String>,
}

impl SoakApp {
    const USAGE: &'static str =
        "soak_monitor DAEMON DATA_DIR SECONDS [SAMPLE_SECS [REPORT_SECS]] [-- DAEMON_ARGS...]";
    const GRACE: Duration = Duration::from_mins(1);

    fn parse() -> Result<Self, String> {
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        let split = arguments.iter().position(|argument| argument == "--");
        let (own, extra) = match split {
            Some(index) => (&arguments[..index], arguments[index + 1..].to_vec()),
            None => (&arguments[..], Vec::new()),
        };
        let [daemon, data_dir, seconds, rest @ ..] = own else {
            return Err(Self::USAGE.to_string());
        };
        let number = |value: Option<&String>, default: u64| -> Result<u64, String> {
            value.map_or(Ok(default), |value| value.parse().map_err(|_| format!("cannot read {value:?}")))
        };
        Ok(Self {
            daemon: PathBuf::from(daemon),
            data_dir: std::path::absolute(data_dir).map_err(|error| format!("{data_dir}: {error}"))?,
            seconds: number(Some(seconds), 0)?,
            sample: Duration::from_secs(number(rest.first(), 5)?.max(1)),
            report: Duration::from_secs(number(rest.get(1), 60)?.max(1)),
            extra,
        })
    }

    fn execute() -> ExitCode {
        match Self::parse().and_then(|app| app.run()) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(error) => {
                eprintln!("soak_monitor: {error}");
                ExitCode::from(2)
            }
        }
    }

    fn log_path(&self) -> PathBuf {
        self.data_dir.with_extension("log")
    }

    fn summary_path(&self) -> PathBuf {
        self.data_dir.with_file_name("soak-summary.txt")
    }

    fn spawn(&self) -> Result<Child, String> {
        std::fs::create_dir_all(&self.data_dir).map_err(|error| format!("{}: {error}", self.data_dir.display()))?;
        Command::new(&self.daemon)
            .arg("--data-dir")
            .arg(&self.data_dir)
            .arg("--log-file")
            .arg(self.log_path())
            .arg("--run-seconds")
            .arg(self.seconds.to_string())
            .args(&self.extra)
            .stdout(Stdio::null())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|error| format!("start {}: {error}", self.daemon.display()))
    }

    /// Returns whether the daemon ran the whole time and stopped cleanly.
    fn run(&self) -> Result<bool, String> {
        let ticks_per_second = rustix::param::clock_ticks_per_second();
        let mut child = self.spawn()?;
        let started = Instant::now();
        let deadline = started + Duration::from_secs(self.seconds) + Self::GRACE;
        let usage = DirUsage::new(&self.data_dir);
        let mut watch = LogWatch::new(&self.log_path());
        let mut report = SoakReport::default();
        let mut next_report = started;
        println!(
            "soak: pid {} for {} s, sampling every {} s, log {}",
            child.id(),
            self.seconds,
            self.sample.as_secs(),
            self.log_path().display()
        );
        let exit = loop {
            if let Some(status) = child.try_wait().map_err(|error| format!("wait: {error}"))? {
                break status;
            }
            if Instant::now() >= deadline {
                eprintln!("soak: daemon still running {} s past its stop time; killing it", Self::GRACE.as_secs());
                let _ = child.kill();
                break child.wait().map_err(|error| format!("wait: {error}"))?;
            }
            if let Ok(sample) = ProcSample::read(child.id()) {
                let disk = usage.bytes();
                report.add(sample, disk, started.elapsed());
                if let Err(error) = watch.poll() {
                    eprintln!("soak: read log: {error}");
                }
                if Instant::now() >= next_report {
                    println!("{}", report.line(disk, ticks_per_second, &watch));
                    next_report += self.report;
                }
            }
            thread::sleep(self.sample);
        };
        let _ = watch.poll();
        let ran_long_enough = started.elapsed() >= Duration::from_secs(self.seconds);
        let summary = report.summary(ticks_per_second, &watch, &exit.to_string());
        println!("\n{summary}");
        std::fs::write(self.summary_path(), &summary)
            .map_err(|error| format!("{}: {error}", self.summary_path().display()))?;
        Ok(exit.success() && ran_long_enough && watch.fatal.is_none())
    }
}

fn main() -> ExitCode {
    SoakApp::execute()
}
