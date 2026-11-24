//! Local, deduplicated diagnostics from content loading and client warnings.

use std::collections::{HashSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::log::tracing::field::{Field, Visit};
use bevy::log::tracing::{Event, Subscriber};
use bevy::log::tracing_subscriber::Layer;
use bevy::log::tracing_subscriber::layer::Context;
use bevy::log::{BoxedLayer, Level};
use bevy::prelude::*;

const RECENT_LIMIT: usize = 4;

#[derive(Resource, Default)]
pub(crate) struct Diagnostics {
    lines: Vec<String>,
    path: Option<PathBuf>,
    revision: usize,
}

impl Diagnostics {
    pub(crate) fn lines(&self) -> &[String] {
        &self.lines
    }

    pub(crate) fn report_path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(crate) fn total(&self) -> usize {
        self.revision
    }
}

#[derive(Default)]
struct Session {
    seen: HashSet<String>,
    recent: VecDeque<String>,
    file: Option<(PathBuf, File)>,
    revision: usize,
}

#[derive(Resource, Clone, Default)]
pub(crate) struct Report(Arc<Mutex<Session>>);

impl Report {
    pub(crate) fn open() -> Self {
        let report = Self::default();
        match open_file(&crate::platform::diagnostics_dir()) {
            Ok((path, file)) => {
                let _ = writeln!(io::stderr(), "STA64 diagnostics: {}", path.display());
                report.0.lock().expect("new report").file = Some((path, file));
            }
            Err(err) => {
                let _ = writeln!(
                    io::stderr(),
                    "STA64 could not open the diagnostics file: {err}"
                );
            }
        }
        report
    }

    pub(crate) fn record(&self, category: &str, detail: &str) {
        self.record_all(category, [detail]);
    }

    pub(crate) fn record_all(
        &self,
        category: &str,
        details: impl IntoIterator<Item = impl AsRef<str>>,
    ) {
        let Ok(mut session) = self.0.lock() else {
            return;
        };
        for detail in details {
            let line = format!("{category} — {}", detail.as_ref())
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect::<String>();
            if !session.seen.insert(line.clone()) {
                continue;
            }
            if let Some((path, file)) = &mut session.file
                && let Err(err) = writeln!(file, "{line}")
            {
                let _ = writeln!(
                    io::stderr(),
                    "STA64 could not write {}: {err}",
                    path.display()
                );
                session.file = None;
            }
            if session.recent.len() == RECENT_LIMIT {
                session.recent.pop_front();
            }
            session.recent.push_back(line);
            session.revision += 1;
        }
    }
}

fn open_file(dir: &Path) -> io::Result<(PathBuf, File)> {
    std::fs::create_dir_all(dir)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let stamp = sta_bench::capture::stamp(now.as_secs());
    let path = dir.join(format!(
        "session-{stamp}-{:09}-{}.txt",
        now.subsec_nanos(),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    writeln!(
        file,
        "STA64 diagnostic log\nVersion: {}\nSession: {stamp} UTC\n",
        crate::platform::diagnostic_build()
    )?;
    Ok((path, file))
}

pub(crate) fn refresh(report: Res<Report>, mut display: ResMut<Diagnostics>) {
    let Ok(session) = report.0.lock() else { return };
    let path = session.file.as_ref().map(|(path, _)| path.as_path());
    if display.revision != session.revision || display.report_path() != path {
        display.lines = session.recent.iter().cloned().collect();
        display.path = path.map(Path::to_owned);
        display.revision = session.revision;
    }
}

/// Persist client warnings that formerly went only to the terminal. Content
/// loader warnings are recorded explicitly since the first mount precedes Bevy.
pub(crate) fn layer(app: &mut App) -> Option<BoxedLayer> {
    Some(Box::new(DiagnosticLayer(
        app.world().get_resource::<Report>()?.clone(),
    )))
}

struct DiagnosticLayer(Report);

impl<S: Subscriber> Layer<S> for DiagnosticLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let meta = event.metadata();
        let category = match *meta.level() {
            Level::ERROR => "Error",
            Level::WARN if meta.target().starts_with("sta") => "Warning",
            _ => return,
        };
        let mut message = EventText::default();
        event.record(&mut message);
        if !message.0.is_empty() {
            self.0.record(category, message.0.trim_end());
        }
    }
}

#[derive(Default)]
struct EventText(String);

impl Visit for EventText {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.push_str(value);
            self.0.push(' ');
        } else {
            self.record_debug(field, &value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write as _;
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?} ");
        } else {
            let _ = write!(self.0, "{}={value:?} ", field.name());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_and_panel_keep_actionable_warnings_without_duplicates() {
        use bevy::log::tracing_subscriber::prelude::*;

        let dir =
            std::env::temp_dir().join(format!("sta64-diagnostics-test-{}", std::process::id()));
        let (path, file) = open_file(&dir.join("logs")).unwrap();
        let report = Report::default();
        report.0.lock().unwrap().file = Some((path.clone(), file));
        std::fs::create_dir_all(dir.join("odf")).unwrap();
        std::fs::write(
            dir.join("odf/gphaser.odf"),
            "classLabel = cannon\nordName = \"missing_beam.odf\"\n",
        )
        .unwrap();
        let mut vfs = sta_vfs::Vfs::default();
        vfs.push_dir("Test Content".into(), dir.clone()).unwrap();
        let mut catalogue = crate::map::Catalogue::with_report(report.clone());
        catalogue.content(&vfs).unwrap();
        catalogue.next_mount().content(&vfs).unwrap();
        let subscriber =
            bevy::log::tracing_subscriber::registry().with(DiagnosticLayer(report.clone()));
        bevy::log::tracing::subscriber::with_default(subscriber, || {
            warn!(target: "sta64::map", "Could not read custom map");
            info!("ordinary startup message");
        });

        let mut app = App::new();
        app.insert_resource(report)
            .init_resource::<Diagnostics>()
            .add_systems(Update, refresh);
        app.update();
        let display = app.world().resource::<Diagnostics>();
        assert_eq!(display.report_path(), Some(path.as_path()));
        let issue = "gphaser.odf: Ordnance ODF unavailable: missing_beam.odf (ordName)";
        assert!(
            display.lines().iter().any(|line| line.contains(issue)),
            "{:?}",
            display.lines()
        );
        assert_eq!(
            display.lines().last().map(String::as_str),
            Some("Warning — Could not read custom map")
        );
        let disk = std::fs::read_to_string(&path).unwrap();
        assert_eq!(disk.matches(issue).count(), 1);
        assert!(disk.contains("Could not read custom map"));
        assert!(!disk.contains("ordinary startup message"));
        drop(app);
        drop(catalogue);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
