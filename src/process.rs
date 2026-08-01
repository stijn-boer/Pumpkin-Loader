use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::OsStr,
    io::{BufRead, BufReader, Read},
    process::{Command, Output, Stdio},
    thread,
    time::Instant,
};

use serde::Deserialize;

use crate::{
    error::{LoaderError, Result},
    logging::BuildProgress,
};

const FAILURE_TAIL_LINES: usize = 80;

pub fn output(command: &mut Command, description: &str) -> Result<String> {
    let result = execute(command, description)?;
    String::from_utf8(result.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|source| LoaderError::CommandOutputUtf8 {
            description: description.to_owned(),
            source,
        })
}

pub fn checked(command: &mut Command, description: &str) -> Result<()> {
    execute(command, description).map(|_| ())
}

pub fn cargo_build(
    command: &mut Command,
    description: &str,
    display_description: &str,
    estimated_packages: usize,
    package_names: &HashMap<String, String>,
) -> Result<()> {
    let rendered = render(command);
    log::debug!("running: {rendered}");

    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|source| LoaderError::CommandSpawn {
            description: description.to_owned(),
            command: rendered.clone(),
            source,
        })?;

    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let stderr_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut reader = BufReader::new(stderr);
        let _ = reader.read_to_end(&mut bytes);
        bytes
    });

    let mut progress = BuildProgress::start(display_description, estimated_packages);
    let mut completed_packages = HashSet::new();
    let mut diagnostics = VecDeque::with_capacity(FAILURE_TAIL_LINES);

    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|source| LoaderError::CommandSpawn {
            description: format!("read output from {description}"),
            command: rendered.clone(),
            source,
        })?;

        if log::log_enabled!(log::Level::Debug) {
            log::debug!("cargo: {line}");
        }

        match serde_json::from_str::<CargoMessage>(&line) {
            Ok(CargoMessage::CompilerArtifact {
                package_id, target, ..
            }) => {
                if completed_packages.insert(package_id.clone()) {
                    let package = package_names
                        .get(&package_id)
                        .map(String::as_str)
                        .unwrap_or_else(|| package_name_from_id(&package_id));
                    progress.package_completed(package, target.name.as_deref());
                }
            }
            Ok(CargoMessage::CompilerMessage { message }) => {
                if let Some(rendered) = message.rendered {
                    push_lines(&mut diagnostics, &rendered);
                }
            }
            Ok(CargoMessage::BuildFinished { .. }) => {}
            Ok(CargoMessage::Other) | Err(_) => {
                if !line.trim().is_empty() {
                    push_line(&mut diagnostics, line);
                }
            }
        }
    }

    let status = child.wait().map_err(|source| LoaderError::CommandSpawn {
        description: description.to_owned(),
        command: rendered.clone(),
        source,
    })?;
    let stderr = stderr_thread.join().unwrap_or_default();
    push_lines(&mut diagnostics, &String::from_utf8_lossy(&stderr));

    if !status.success() {
        return Err(LoaderError::CommandFailed {
            description: description.to_owned(),
            command: rendered,
            status,
            diagnostic: indent(&diagnostics.into_iter().collect::<Vec<_>>().join("\n")),
        });
    }

    progress.finish();
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
enum CargoMessage {
    CompilerArtifact {
        package_id: String,
        target: CargoTarget,
    },
    CompilerMessage {
        message: CargoDiagnostic,
    },
    BuildFinished,
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct CargoTarget {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CargoDiagnostic {
    rendered: Option<String>,
}

fn package_name_from_id(package_id: &str) -> &str {
    package_id
        .split([' ', '#'])
        .find(|part| !part.contains('/') && !part.contains(':'))
        .unwrap_or(package_id)
}

fn push_lines(lines: &mut VecDeque<String>, text: &str) {
    for line in text.lines() {
        push_line(lines, line.to_owned());
    }
}

fn push_line(lines: &mut VecDeque<String>, line: String) {
    if lines.len() == FAILURE_TAIL_LINES {
        lines.pop_front();
    }
    lines.push_back(line);
}

fn execute(command: &mut Command, description: &str) -> Result<Output> {
    let rendered = render(command);
    log::debug!("running: {rendered}");
    let started = Instant::now();

    let result = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|source| LoaderError::CommandSpawn {
            description: description.to_owned(),
            command: rendered.clone(),
            source,
        })?;

    if log::log_enabled!(log::Level::Debug) {
        emit_debug("stdout", &result.stdout);
        emit_debug("stderr", &result.stderr);
    }

    if !result.status.success() {
        return Err(LoaderError::CommandFailed {
            description: description.to_owned(),
            command: rendered,
            status: result.status,
            diagnostic: indent(&combined_tail(&result)),
        });
    }

    log::debug!("{description} completed in {:.2?}", started.elapsed());
    Ok(result)
}

pub fn render(command: &Command) -> String {
    let mut parts = vec![quote(command.get_program())];
    parts.extend(command.get_args().map(quote));
    parts.join(" ")
}

fn quote(value: &OsStr) -> String {
    let text = value.to_string_lossy();
    if text
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-_=./:@+".contains(character))
    {
        text.into_owned()
    } else {
        format!("{text:?}")
    }
}

fn emit_debug(stream: &str, bytes: &[u8]) {
    for line in String::from_utf8_lossy(bytes).lines() {
        log::debug!("{stream}: {line}");
    }
}

fn combined_tail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let selected = if stderr.trim().is_empty() {
        stdout.as_ref()
    } else {
        stderr.as_ref()
    };
    let lines: Vec<_> = selected.lines().collect();
    let start = lines.len().saturating_sub(FAILURE_TAIL_LINES);
    let prefix = if start > 0 {
        format!("... {start} earlier lines omitted ...\n")
    } else {
        String::new()
    };
    format!("{prefix}{}", lines[start..].join("\n"))
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}
