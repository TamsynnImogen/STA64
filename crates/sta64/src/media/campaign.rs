//! Mission movies in the campaign flow (MED-04).
//!
//! # The rule, and where it comes from
//!
//! A campaign mission owns no movie in any table. What ties a movie to a mission is the
//! mission script: the Armada I `*S.dsl` (and Armada II's) names its `.bik` files and calls
//! `PlayBridgeMovie(name, flag)` (Armada I, the bridge viewscreen) or
//! `PlayCinematicMovie(script, name, ..)` (Armada II, full screen) when a story event of the
//! mission fires (a distress call, an objective met, an arrival), never unconditionally at the
//! start. So a movie plays when its script asks for it, which is [`super::mission::bridge`]'s
//! job (the `Present::Movie` events of LUA-06 / LUA-05). This module adds what the campaign
//! flow needs around that:
//!
//! * [`MissionMovies`]: which movies each campaign mission's script names
//!   ([`CampaignMission::movies`](sta_mission::campaign::CampaignMission), read from data) and
//!   which of them the install has, for the briefing, the debrief and a missing-file report.
//! * [`MissionMovieLog`]: what the running mission asked for and how each request ended, so a
//!   screen can wait for the bridge scene to be over ([`MissionMovieLog::busy`]) before it
//!   shows the briefing or the debrief.
//! * [`MissionBegan`] / [`MissionEnded`]: the two messages the campaign flow (UI-25, INT-M6)
//!   writes. Ending a mission stops any of its movies still running, so a skipped or lost
//!   mission never leaves a bridge scene over the debrief.
//!
//! Skipping is [`super::skip_keys`]'s: Esc, Space, Enter or a click ends a fullscreen movie,
//! and a script-requested movie always reports [`MovieFinished`], so the script (and the
//! flow) goes on. Not in this module, by decision of the plan: the campaign screens (UI-25)
//! and the script-side reply (UI-30).
//!
//! Evidence grade: the mission-to-movie association and the call that plays each are read
//! from the scripts' strings and imports (static, executable-level; Armada I and II); the
//! story events that trigger them are the scripts' own and are not modelled here.

use bevy::prelude::*;
use sta_mission::campaign::{CampaignBook, CampaignMission};
use sta_vfs::Vfs;

use super::mission::Requested;
use super::{MovieFinished, MovieId, MovieOutcome, StopMovie, locate};
use crate::app::GameSet;

/// One movie a mission script names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissionMovie {
    /// As the script spells it (`F1DEM1.bik`).
    pub name: String,
    /// The VFS path [`locate`] would open first, or `None` when the install has no such file.
    pub path: Option<String>,
}

/// The movies of one mission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissionEntry {
    pub campaign: String,
    /// Position in its campaign.
    pub index: usize,
    /// Lower-case map stem (`fed1`).
    pub stem: String,
    pub movies: Vec<MissionMovie>,
}

/// Every campaign mission's movies for one install, built when the campaign book is.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct MissionMovies {
    pub missions: Vec<MissionEntry>,
}

/// A map name (`fed1.bzn`, `bzn/Fed1.bzn`, `FED1`) as the stem the catalogue keys on.
pub fn stem_of(map: &str) -> String {
    let f = map.rsplit(['/', '\\']).next().unwrap_or(map);
    let f = if f.len() > 4 && f[f.len() - 4..].eq_ignore_ascii_case(".bzn") { &f[..f.len() - 4] } else { f };
    f.to_ascii_lowercase()
}

impl MissionMovies {
    /// Read every mission of `book` and look its movies up in `vfs`.
    pub fn from_book(book: &CampaignBook, vfs: &Vfs) -> Self {
        Self::from_book_with(book, |name| locate::candidates(name).into_iter().find(|p| vfs.resolve(p).is_some()))
    }

    /// As [`from_book`](Self::from_book) with the lookup given (tests).
    pub fn from_book_with(book: &CampaignBook, find: impl Fn(&str) -> Option<String>) -> Self {
        let mut missions = Vec::new();
        for c in &book.campaigns {
            for m in &c.missions {
                missions.push(entry(&c.id, m, &find));
            }
        }
        Self { missions }
    }

    /// The movies of the mission whose map is `map`, wherever it sits.
    pub fn for_map(&self, map: &str) -> &[MissionMovie] {
        let stem = stem_of(map);
        self.missions.iter().find(|e| e.stem == stem).map_or(&[], |e| &e.movies)
    }

    /// The mission that names `movie` first (case-insensitive, path ignored).
    pub fn owner_of(&self, movie: &str) -> Option<&MissionEntry> {
        let want = file_of(movie);
        self.missions.iter().find(|e| e.movies.iter().any(|m| file_of(&m.name) == want))
    }

    /// `(map stem, movie)` pairs a script names that the install lacks.
    pub fn missing(&self) -> Vec<(&str, &str)> {
        self.missions.iter().flat_map(|e| e.movies.iter().filter(|m| m.path.is_none()).map(|m| (e.stem.as_str(), m.name.as_str()))).collect()
    }
}

fn file_of(name: &str) -> String {
    name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase()
}

fn entry(campaign: &str, m: &CampaignMission, find: &impl Fn(&str) -> Option<String>) -> MissionEntry {
    MissionEntry {
        campaign: campaign.to_owned(),
        index: m.index,
        stem: stem_of(&m.bzn),
        movies: m.movies.iter().map(|n| MissionMovie { name: n.clone(), path: find(n) }).collect(),
    }
}

/// The campaign flow starts a mission (written by UI-25 / INT-M6).
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct MissionBegan {
    /// The mission's map (`fed1.bzn`).
    pub map: String,
}

/// The mission is over (won, lost or abandoned); its movies stop.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissionEnded;

/// One movie request of the running mission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedMovie {
    pub id: MovieId,
    pub name: String,
    /// The mission's script names this movie (false: a script asked for one the book does not list).
    pub listed: bool,
    /// `None` while it waits or plays.
    pub outcome: Option<MovieOutcome>,
}

/// What the running mission asked to play and how it ended.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct MissionMovieLog {
    /// Stem of the running mission; `None` between missions.
    pub mission: Option<String>,
    pub movies: Vec<LoggedMovie>,
}

impl MissionMovieLog {
    /// A requested movie has not ended yet: a briefing or debrief waits for this to be false.
    pub fn busy(&self) -> bool {
        self.movies.iter().any(|m| m.outcome.is_none())
    }

    /// Movies that ended with the given outcome kind.
    pub fn count(&self, f: impl Fn(&MovieOutcome) -> bool) -> usize {
        self.movies.iter().filter(|m| m.outcome.as_ref().is_some_and(&f)).count()
    }
}

/// Adds the log and the two messages; [`MediaPlugin`](super::MediaPlugin) adds it.
pub struct CampaignMoviesPlugin;

impl Plugin for CampaignMoviesPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MissionBegan>()
            .add_message::<MissionEnded>()
            .init_resource::<MissionMovieLog>()
            .init_resource::<MissionMovies>()
            // After `drive`, so a request and its end in one frame are both seen in order.
            .add_systems(Update, track.after(super::drive).in_set(GameSet::Ui));
    }
}

fn track(
    mut began: MessageReader<MissionBegan>,
    mut ended: MessageReader<MissionEnded>,
    mut requested: MessageReader<Requested>,
    mut finished: MessageReader<MovieFinished>,
    catalogue: Res<MissionMovies>,
    mut log: ResMut<MissionMovieLog>,
    mut stop: MessageWriter<StopMovie>,
) {
    for b in began.read() {
        let stem = stem_of(&b.map);
        let list = catalogue.for_map(&b.map);
        let absent: Vec<&str> = list.iter().filter(|m| m.path.is_none()).map(|m| m.name.as_str()).collect();
        info!("media: mission {stem} names {} movie(s){}", list.len(), if absent.is_empty() { String::new() } else { format!(", not installed: {absent:?}") });
        *log = MissionMovieLog { mission: Some(stem), movies: Vec::new() };
    }
    for r in requested.read() {
        if log.mission.is_none() {
            continue;
        }
        let listed = log.mission.as_deref().is_some_and(|m| catalogue.for_map(m).iter().any(|x| file_of(&x.name) == file_of(&r.name)));
        log.movies.push(LoggedMovie { id: r.id, name: r.name.clone(), listed, outcome: None });
    }
    for f in finished.read() {
        if let Some(m) = log.movies.iter_mut().find(|m| m.id == f.id) {
            m.outcome = Some(f.outcome.clone());
        }
    }
    if ended.read().count() > 0 {
        if log.busy() {
            stop.write(StopMovie(None));
        }
        log.mission = None;
    }
}

#[cfg(test)]
mod tests {
    use sta_mission::campaign::ShellLayout;

    use super::*;

    use sta_mission::campaign::{Campaign, MissionScript};

    fn mission(index: usize, bzn: &str, movies: &[&str]) -> CampaignMission {
        CampaignMission {
            index,
            bzn: bzn.into(),
            title: None,
            map_present: true,
            script: MissionScript::None,
            objective_files: vec![],
            briefing: None,
            movies: movies.iter().map(|s| (*s).into()).collect(),
            end_calls: vec![],
        }
    }

    #[test]
    fn from_book_keeps_script_order_and_looks_each_movie_up() {
        let book = CampaignBook {
            layout: ShellLayout::Armada1,
            campaigns: vec![Campaign {
                id: "federation".into(),
                slot: 0,
                name: None,
                missions: vec![mission(0, "fed1.bzn", &["F1DEM1.bik", "F1DEM2.bik"]), mission(1, "fed2.bzn", &[])],
                requires: None,
                locked_text: None,
            }],
            give_all_missions: false,
            warnings: vec![],
        };
        let cat = MissionMovies::from_book_with(&book, |n| (n != "F1DEM2.bik").then(|| format!("animations/{}", n.to_ascii_lowercase())));
        assert_eq!(cat.missions.len(), 2);
        let names: Vec<_> = cat.for_map("fed1.bzn").iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["F1DEM1.bik", "F1DEM2.bik"]);
        assert_eq!(cat.for_map("fed1.bzn")[0].path.as_deref(), Some("animations/f1dem1.bik"));
        assert_eq!(cat.missing(), [("fed1", "F1DEM2.bik")]);
        assert!(cat.for_map("fed2.bzn").is_empty());
    }

    #[test]
    fn stems_ignore_case_dirs_and_extension() {
        assert_eq!(stem_of("bzn/Fed1.BZN"), "fed1");
        assert_eq!(stem_of("fed1"), "fed1");
    }

    #[test]
    fn catalogue_lists_script_movies_and_what_is_missing() {
        let cat = MissionMovies {
            missions: vec![MissionEntry {
                campaign: "federation".into(),
                index: 0,
                stem: "fed1".into(),
                movies: vec![
                    MissionMovie { name: "F1DEM1.bik".into(), path: Some("animations/f1dem1.bik".into()) },
                    MissionMovie { name: "gone.bik".into(), path: None },
                ],
            }],
        };
        assert_eq!(cat.for_map("FED1.bzn").len(), 2);
        assert!(cat.for_map("other.bzn").is_empty());
        assert_eq!(cat.missing(), [("fed1", "gone.bik")]);
        assert_eq!(cat.owner_of("animations\\f1dem1.BIK").map(|e| e.stem.as_str()), Some("fed1"));
        assert!(cat.owner_of("x.bik").is_none());
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<Requested>()
            .add_message::<MovieFinished>()
            .add_message::<StopMovie>()
            .add_plugins(CampaignMoviesPlugin);
        crate::app::configure_sets(&mut app);
        app.world_mut().insert_resource(MissionMovies {
            missions: vec![MissionEntry {
                campaign: "federation".into(),
                index: 0,
                stem: "fed1".into(),
                movies: vec![MissionMovie { name: "F1DEM1.bik".into(), path: None }],
            }],
        });
        app
    }

    fn stops(app: &mut App) -> usize {
        app.world_mut().resource_mut::<Messages<StopMovie>>().drain().count()
    }

    #[test]
    fn the_log_follows_a_missions_movie_requests_to_their_end() {
        let mut app = app();
        app.world_mut().write_message(Requested { id: MovieId(1), name: "f1dem1.bik".into() });
        app.update();
        assert!(app.world().resource::<MissionMovieLog>().movies.is_empty(), "no mission, nothing logged");
        app.world_mut().write_message(MissionBegan { map: "fed1.bzn".into() });
        app.update();
        app.world_mut().write_message(Requested { id: MovieId(2), name: "f1dem1.bik".into() });
        app.world_mut().write_message(Requested { id: MovieId(3), name: "surprise.bik".into() });
        app.update();
        let log = app.world().resource::<MissionMovieLog>().clone();
        assert_eq!(log.mission.as_deref(), Some("fed1"));
        assert_eq!(log.movies.iter().map(|m| m.listed).collect::<Vec<_>>(), [true, false]);
        assert!(log.busy());
        app.world_mut().write_message(MovieFinished { id: MovieId(2), name: "f1dem1.bik".into(), outcome: MovieOutcome::Skipped });
        app.world_mut().write_message(MovieFinished { id: MovieId(3), name: "surprise.bik".into(), outcome: MovieOutcome::Failed("x".into()) });
        app.update();
        let log = app.world().resource::<MissionMovieLog>();
        assert!(!log.busy());
        assert_eq!((log.count(|o| *o == MovieOutcome::Skipped), log.count(|o| matches!(o, MovieOutcome::Failed(_)))), (1, 1));
        assert_eq!(stops(&mut app), 0, "nothing left to stop");
    }

    #[test]
    fn ending_a_mission_stops_its_running_movies() {
        let mut app = app();
        app.world_mut().write_message(MissionBegan { map: "fed1.bzn".into() });
        app.world_mut().write_message(Requested { id: MovieId(1), name: "F1DEM1.bik".into() });
        app.update();
        app.world_mut().write_message(MissionEnded);
        app.update();
        assert_eq!(stops(&mut app), 1);
        assert_eq!(app.world().resource::<MissionMovieLog>().mission, None);
    }

    /// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`): the movies each campaign mission's script names,
    /// and whether the install has them.
    #[test]
    fn install_mission_movies_resolve() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
        let m = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
        let vfs = Vfs::mount_install(std::path::Path::new(&root), m.as_deref()).expect("mount");
        let book = CampaignBook::load_vfs(&vfs);
        let cat = MissionMovies::from_book(&book, &vfs);
        let total: usize = cat.missions.iter().map(|e| e.movies.len()).sum();
        eprintln!("{root}: {} missions, {total} script movies, missing {:?}", cat.missions.len(), cat.missing());
        assert!(cat.missing().is_empty(), "scripts name movies the install lacks: {:?}", cat.missing());
    }
}
