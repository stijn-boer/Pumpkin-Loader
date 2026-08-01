use std::{
    io::{self, Write},
    sync::OnceLock,
    time::{Duration, Instant},
};

use env_logger::{Builder, Env, Target};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use log::LevelFilter;

static PROGRESS: OnceLock<MultiProgress> = OnceLock::new();

fn multi_progress() -> &'static MultiProgress {
    PROGRESS.get_or_init(MultiProgress::new)
}

/// Initializes `env_logger` while preserving the CLI's `-q`/`-v` behavior.
///
/// `RUST_LOG` takes precedence when set. Otherwise:
/// - `-q` logs errors only
/// - no flag logs info and above
/// - `-v` logs debug and above
/// - `-vv` logs trace and above
pub fn init(verbose: u8, quiet: bool) {
    let default_level = if quiet {
        LevelFilter::Error
    } else {
        match verbose {
            0 => LevelFilter::Info,
            1 => LevelFilter::Debug,
            _ => LevelFilter::Trace,
        }
    };

    let mut builder = Builder::from_env(Env::default().default_filter_or(default_level.as_str()));

    builder
        .target(Target::Pipe(Box::new(IndicatifWriter)))
        .format_timestamp_secs()
        .format_target(false)
        .format_module_path(false);

    builder.init();
}

fn progress_enabled() -> bool {
    log::max_level() > LevelFilter::Error
}

#[derive(Debug)]
struct IndicatifWriter;

impl Write for IndicatifWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buffer);
        for line in text.lines() {
            multi_progress().println(line).map_err(io::Error::other)?;
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub struct Operation {
    description: String,
    started: Instant,
    progress: ProgressBar,
    finished: bool,
}

impl Operation {
    pub fn start(description: impl Into<String>) -> Self {
        let description = description.into();
        let progress = if progress_enabled() {
            let progress = multi_progress().add(ProgressBar::new_spinner());
            progress.set_style(
                ProgressStyle::with_template("{spinner:.cyan} {msg} [{elapsed_precise}]")
                    .expect("static operation template is valid")
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
            );
            progress.set_message(description.clone());
            progress.enable_steady_tick(Duration::from_millis(100));
            progress
        } else {
            ProgressBar::hidden()
        };

        Self {
            description,
            started: Instant::now(),
            progress,
            finished: false,
        }
    }

    pub fn finish(mut self) {
        self.finished = true;
        if progress_enabled() {
            self.progress.finish_with_message(format!(
                "{} completed in {}",
                self.description,
                format_duration(self.started.elapsed())
            ));
        }
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        if !self.finished {
            self.progress.abandon_with_message(format!(
                "{} failed after {}",
                self.description,
                format_duration(self.started.elapsed())
            ));
        }
    }
}

pub struct BuildProgress {
    description: String,
    started: Instant,
    progress: ProgressBar,
    estimated_total: u64,
    completed: u64,
    finished: bool,
}

impl BuildProgress {
    pub fn start(description: impl Into<String>, estimated_total: usize) -> Self {
        let description = description.into();
        let estimated_total = estimated_total.max(1) as u64;
        let progress = if progress_enabled() {
            let progress = multi_progress().add(ProgressBar::new(estimated_total));
            progress.set_style(
                ProgressStyle::with_template(
                    "{spinner:.cyan} [{bar:32.cyan/blue}] {percent:>3}% {pos}/{len} {msg} [{elapsed_precise}]",
                )
                .expect("static build progress template is valid")
                .progress_chars("=>-")
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
            );
            progress.set_message(description.clone());
            progress.enable_steady_tick(Duration::from_millis(100));
            progress
        } else {
            ProgressBar::hidden()
        };

        Self {
            description,
            started: Instant::now(),
            progress,
            estimated_total,
            completed: 0,
            finished: false,
        }
    }

    pub fn package_completed(&mut self, package: &str, target: Option<&str>) {
        self.completed = self
            .completed
            .saturating_add(1)
            .min(self.estimated_total.saturating_sub(1));
        self.progress.set_position(self.completed);
        self.set_current(package, target);
    }

    pub fn set_current(&self, package: &str, target: Option<&str>) {
        let message = match target {
            Some(target) if target != package => format!("Finished {package} ({target})"),
            _ => format!("Finished {package}"),
        };
        self.progress.set_message(message);
    }

    pub fn finish(mut self) {
        self.finished = true;
        self.progress.set_position(self.estimated_total);
        if progress_enabled() {
            self.progress.finish_with_message(format!(
                "{} completed in {}",
                self.description,
                format_duration(self.started.elapsed())
            ));
        }
    }
}

impl Drop for BuildProgress {
    fn drop(&mut self) {
        if !self.finished {
            self.progress.abandon_with_message(format!(
                "{} failed after {}",
                self.description,
                format_duration(self.started.elapsed())
            ));
        }
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;

    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}
