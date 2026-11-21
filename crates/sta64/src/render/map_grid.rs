//! The map grid (#1033): a fine line grid over the map's plane.
//!
//! Rule STA64 follows (grades: read from the executable, static, unless said otherwise; the
//! per-install defaults: read from data; that a line adds the same dim colour over every
//! background, is one device pixel wide, does not fade with distance and is drawn under units: measured
//! on one install):
//! - One line every [`GRID_SIZE`] world units along each axis across the map rectangle, counted from the
//!   map's low corner, so a map whose extent is not a multiple of the cell stops short of the far edge.
//!   The plane is the map's ground plane (height 0).
//! - The lines are added to the picture in a dim constant colour ([`GRID_COLOUR`]): additive, depth tested
//!   (units draw over it), no depth write, one device pixel wide at any zoom, no fade. The fog and shroud
//!   layers sort after it and cover it. Depth write: the original's is not established; additive lines
//!   need none.
//! - Per view the grid is shown or not by a mode, 0 grid only, 1 grid with height bars, 2 none. The first
//!   value comes from the mounted `RTS_CFG.h` (`cfgSTRATEGIC_GRID_RENDER` for the original's normal
//!   view, `cfgTACTICAL_GRID_RENDER` for its tactical view), read from data; the grid-toggle action
//!   cycles 0, 1, 2 for the current view. A definition that declares "none" shows no grid until toggled
//!   (ADR 0004 D9: the typed rule comes from the legacy cfg, no new rules file; the keys are client-only
//!   and not part of the resolved sim rules of D10).
//! - The cell size and the colour are not cfg keys: code constants, so a mod cannot change them.
//!
//! Interim (marked, see the PR): STA64 has one orbit camera, mapped to the original's normal view; the
//! tactical view has no camera here, its key is read and kept. The height bars of mode 1 are STA64's own
//! strategic-view bars (drawn by `render::strategic` whenever that view is active), so mode 1 draws
//! the grid as mode 0 does. The key binding is STA64's own (Alt+G, the `input.map` default of the
//! original, which STA64 does not read). The mission call that sets grid visibility stays unwired.
//!
//! One mesh: a line list rebuilt only when the map bounds change, one entity, one draw.
//! Both looks draw it the same way; a look's post chain (tone mapping, bloom) may change the dim colour
//! on screen and is not tuned here.

use crate::app::GameSet;
use crate::input::keys::{Action, KeyBindings};
use crate::render::fog::FogBounds;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;

/// World units between two lines (read from the executable, static; no data key).
pub const GRID_SIZE: f32 = 100.0;
/// The lines' colour, added in framebuffer terms (read from the executable, static; no data key).
pub const GRID_COLOUR: [f32; 3] = [0.149, 0.128, 0.0];
/// `RTS_CFG.h` key of the original's normal view's initial mode (read from data).
pub const STRATEGIC_KEY: &str = "cfgSTRATEGIC_GRID_RENDER";
/// `RTS_CFG.h` key of the original's tactical view's initial mode (read from data).
pub const TACTICAL_KEY: &str = "cfgTACTICAL_GRID_RENDER";

/// What a view draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GridMode {
    /// 0: the grid.
    Grid,
    /// 1: the grid and height bars (interim: as `Grid`, see the module docs).
    GridAndBars,
    /// 2: nothing.
    #[default]
    None,
}

impl GridMode {
    /// A cfg value; anything outside 0..=2 is "none".
    pub fn from_value(v: i64) -> GridMode {
        match v {
            0 => GridMode::Grid,
            1 => GridMode::GridAndBars,
            _ => GridMode::None,
        }
    }

    /// The next mode of the toggle: 0, 1, 2, 0.
    pub fn next(self) -> GridMode {
        match self {
            GridMode::Grid => GridMode::GridAndBars,
            GridMode::GridAndBars => GridMode::None,
            GridMode::None => GridMode::Grid,
        }
    }

    pub fn shown(self) -> bool {
        self != GridMode::None
    }
}

/// The mode of each of the original's views. A cfg with no such key gives "none" (interim: the key is
/// absent in a definition the grid default of which is not established).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GridModes {
    /// The normal RTS view: STA64's orbit camera (interim mapping).
    pub rts: GridMode,
    /// The tactical view: no camera of STA64's maps to it; kept so the key is read.
    pub tactical: GridMode,
}

impl GridModes {
    pub fn from_cfg(cfg: Option<&sta_assets::cfg_h::CfgH>) -> GridModes {
        let read = |k: &str| cfg.and_then(|c| c.i64(k)).map_or(GridMode::None, GridMode::from_value);
        GridModes { rts: read(STRATEGIC_KEY), tactical: read(TACTICAL_KEY) }
    }
}

/// Line positions along one axis from `lo` to `hi`: `ftol((hi - lo) / cell) + 1` of them, each a cell after the
/// last from `lo`.
pub fn line_positions(lo: f32, hi: f32) -> Vec<f32> {
    let n = ((hi - lo) / GRID_SIZE).max(0.0) as usize + 1;
    (0..n).map(|i| lo + i as f32 * GRID_SIZE).collect()
}

/// The line list for a map rectangle (sim `x`, `z`; render space mirrors Z), as vertex pairs.
pub fn grid_vertices(min: Vec2, max: Vec2) -> Vec<[f32; 3]> {
    let (xs, zs) = (line_positions(min.x, max.x), line_positions(min.y, max.y));
    let mut v = Vec::with_capacity((xs.len() + zs.len()) * 2);
    for x in xs {
        v.push([x, 0.0, -min.y]);
        v.push([x, 0.0, -max.y]);
    }
    for z in zs {
        v.push([min.x, 0.0, -z]);
        v.push([max.x, 0.0, -z]);
    }
    v
}

fn grid_mesh(b: &FogBounds) -> Mesh {
    let v = grid_vertices(b.min, b.max);
    let n = v.len();
    Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, v)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; n])
}

/// The grid's one entity; `bounds` is what its mesh was built for, `builds` how many times a mesh was built.
#[derive(Component, Debug)]
pub struct MapGrid {
    pub bounds: (Vec2, Vec2),
    pub builds: u32,
}

/// Cycle the current view's mode on the grid-toggle chord (the reveal modifier and the grid key).
fn toggle(mut modes: ResMut<GridModes>, keys: Option<Res<ButtonInput<KeyCode>>>, bindings: Res<KeyBindings>) {
    let Some(keys) = keys else { return };
    if bindings.pressed(Action::RevealModifier, &keys) && bindings.just_pressed(Action::GridToggle, &keys) {
        modes.rts = modes.rts.next();
    }
}

/// Keep the one mesh and entity current and show or hide it by the mode.
#[allow(clippy::too_many_arguments)]
fn update(
    mut commands: Commands,
    modes: Res<GridModes>,
    bounds: Option<Res<FogBounds>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut grid: Query<(&mut MapGrid, &mut Mesh3d, &mut Visibility)>,
) {
    let want = modes.rts.shown() && bounds.is_some();
    let Some(bounds) = bounds else { return };
    let now = (bounds.min, bounds.max);
    if let Ok((mut g, mut mesh, mut vis)) = grid.single_mut() {
        if g.bounds != now {
            g.bounds = now;
            g.builds += 1;
            mesh.0 = meshes.add(grid_mesh(&bounds));
        }
        let v = if want { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != v {
            *vis = v;
        }
        return;
    }
    // Additive, unlit, depth tested and not written (alpha-added materials never write). The colour is the
    // framebuffer's own, so it goes in as sRGB.
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(GRID_COLOUR[0], GRID_COLOUR[1], GRID_COLOUR[2]),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        ..default()
    });
    commands.spawn((
        MapGrid { bounds: now, builds: 1 },
        Mesh3d(meshes.add(grid_mesh(&bounds))),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        if want { Visibility::Inherited } else { Visibility::Hidden },
        bevy::camera::visibility::NoFrustumCulling,
        crate::map::MatchEntity,
    ));
}

pub struct MapGridPlugin;

impl Plugin for MapGridPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GridModes>().init_resource::<KeyBindings>().add_systems(
            Update,
            (
                toggle.in_set(GameSet::Input).run_if(crate::game::flow::match_running).run_if(crate::ui::ingame_menu::menu_closed).run_if(crate::cheats::input::game_input_free),
                update.in_set(GameSet::Render).run_if(resource_exists::<Assets<Mesh>>).run_if(resource_exists::<Assets<StandardMaterial>>).run_if(not(resource_exists::<crate::frontend::scene::MenuScene>)),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(strategic: Option<i64>, tactical: Option<i64>) -> sta_assets::cfg_h::CfgH {
        let mut s = String::new();
        if let Some(v) = strategic {
            s += &format!("int {STRATEGIC_KEY} = {v};\n");
        }
        if let Some(v) = tactical {
            s += &format!("int {TACTICAL_KEY} = {v};\n");
        }
        sta_assets::cfg_h::CfgH::parse(&s)
    }

    #[test]
    fn a_whole_number_of_cells_gets_a_line_on_the_far_edge() {
        let xs = line_positions(-300.0, 300.0);
        assert_eq!(xs.len(), 7);
        assert_eq!(xs.first(), Some(&-300.0));
        assert_eq!(xs.last(), Some(&300.0));
    }

    #[test]
    fn a_map_that_is_not_a_multiple_of_the_cell_stops_short_of_the_far_edge() {
        let xs = line_positions(-250.0, 330.0);
        // 580 / 100 -> 5, so 6 lines: -250 .. 250; the edge at 330 has none.
        assert_eq!(xs, vec![-250.0, -150.0, -50.0, 50.0, 150.0, 250.0]);
        // A map narrower than a cell: its low edge only.
        assert_eq!(line_positions(10.0, 60.0), vec![10.0]);
    }

    #[test]
    fn the_vertices_are_one_pair_per_line_spanning_the_whole_rectangle_in_render_space() {
        let (min, max) = (Vec2::new(0.0, 0.0), Vec2::new(200.0, 150.0));
        let v = grid_vertices(min, max);
        // 3 lines of constant x and 2 of constant z.
        assert_eq!(v.len(), (3 + 2) * 2);
        assert!(v.iter().all(|p| p[1] == 0.0));
        // The first line: x = 0 from z = 0 to z = 150, the sim's z mirrored.
        assert_eq!(v[0], [0.0, 0.0, 0.0]);
        assert_eq!(v[1], [0.0, 0.0, -150.0]);
        // The last: z = 100 from x = 0 to x = 200.
        assert_eq!(v[8], [0.0, 0.0, -100.0]);
        assert_eq!(v[9], [200.0, 0.0, -100.0]);
    }

    #[test]
    fn the_mode_cycles_and_cfg_values_give_the_initial_modes() {
        assert_eq!(GridMode::Grid.next(), GridMode::GridAndBars);
        assert_eq!(GridMode::GridAndBars.next(), GridMode::None);
        assert_eq!(GridMode::None.next(), GridMode::Grid);
        let on = GridModes::from_cfg(Some(&cfg(Some(0), Some(2))));
        assert_eq!((on.rts, on.tactical), (GridMode::Grid, GridMode::None));
        let bars = GridModes::from_cfg(Some(&cfg(Some(1), Some(0))));
        assert_eq!((bars.rts, bars.tactical), (GridMode::GridAndBars, GridMode::Grid));
        // "None" shows no grid until toggled.
        let none = GridModes::from_cfg(Some(&cfg(Some(2), Some(2))));
        assert!(!none.rts.shown() && !none.tactical.shown());
        assert_eq!(none.rts.next(), GridMode::Grid);
        // No key, no file: no grid (interim).
        assert_eq!(GridModes::from_cfg(Some(&cfg(None, None))), GridModes::default());
        assert_eq!(GridModes::from_cfg(None), GridModes::default());
    }

    fn rig(modes: GridModes) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .insert_resource(modes)
            
            .insert_resource(FogBounds { min: Vec2::new(0.0, 0.0), max: Vec2::new(1000.0, 450.0) })
            .add_systems(Update, update);
        app.update();
        app
    }

    fn grids(app: &mut App) -> Vec<(u32, Visibility)> {
        app.world_mut().query::<(&MapGrid, &Visibility)>().iter(app.world()).map(|(g, v)| (g.builds, *v)).collect()
    }

    #[test]
    fn one_entity_one_mesh_rebuilt_only_when_the_bounds_change() {
        let mut app = rig(GridModes { rts: GridMode::Grid, tactical: GridMode::None });
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(grids(&mut app), vec![(1, Visibility::Inherited)]);
        let mesh = app.world_mut().query::<&Mesh3d>().single(app.world()).unwrap().0.clone();
        let verts = app.world().resource::<Assets<Mesh>>().get(&mesh).unwrap().count_vertices();
        // 11 lines of constant x and 5 of constant z.
        assert_eq!(verts, (11 + 5) * 2);
        assert_eq!(app.world().resource::<Assets<Mesh>>().get(&mesh).unwrap().primitive_topology(), PrimitiveTopology::LineList);
        app.world_mut().insert_resource(FogBounds { min: Vec2::new(0.0, 0.0), max: Vec2::new(300.0, 300.0) });
        app.update();
        assert_eq!(grids(&mut app), vec![(2, Visibility::Inherited)]);
        let mesh = app.world_mut().query::<&Mesh3d>().single(app.world()).unwrap().0.clone();
        assert_eq!(app.world().resource::<Assets<Mesh>>().get(&mesh).unwrap().count_vertices(), (4 + 4) * 2);
    }

    #[test]
    fn the_mode_shows_and_hides_the_one_entity_in_every_look() {
        use crate::render::profile::RenderProfile;
        for look in [RenderProfile::Stock, RenderProfile::Refit] {
            let mut app = rig(GridModes { rts: GridMode::None, tactical: GridMode::Grid });
            app.insert_resource(look);
            app.update();
            assert_eq!(grids(&mut app), vec![(1, Visibility::Hidden)], "{look:?}");
            app.world_mut().resource_mut::<GridModes>().rts = GridMode::Grid;
            app.update();
            assert_eq!(grids(&mut app), vec![(1, Visibility::Inherited)], "{look:?}");
                app.world_mut().resource_mut::<GridModes>().rts = GridMode::GridAndBars;
            app.update();
            assert_eq!(grids(&mut app), vec![(1, Visibility::Inherited)], "{look:?}");
        }
    }

    #[test]
    fn the_toggle_chord_cycles_the_current_view() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<KeyBindings>()
            .insert_resource(GridModes { rts: GridMode::None, tactical: GridMode::None })
            .init_resource::<ButtonInput<KeyCode>>()
            .add_systems(Update, toggle);
        let press = |app: &mut App, keys: &[KeyCode]| {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.release_all();
            input.clear();
            for k in keys {
                input.press(*k);
            }
        };
        // The key alone does nothing.
        press(&mut app, &[KeyCode::KeyG]);
        app.update();
        assert_eq!(app.world().resource::<GridModes>().rts, GridMode::None);
        // Alt+G: 2 -> 0 -> 1 -> 2.
        for want in [GridMode::Grid, GridMode::GridAndBars, GridMode::None] {
            press(&mut app, &[KeyCode::AltLeft, KeyCode::KeyG]);
            app.update();
            assert_eq!(app.world().resource::<GridModes>().rts, want);
        }
        assert_eq!(app.world().resource::<GridModes>().tactical, GridMode::None);
    }

    #[test]
    fn the_colour_and_cell_are_constants_and_the_bindings_name_the_action() {
        assert_eq!(GRID_SIZE, 100.0);
        assert!(GRID_COLOUR.iter().all(|c| (0.0..0.2).contains(c)));
        assert_eq!(KeyBindings::default().keys(Action::GridToggle), [KeyCode::KeyG]);
        assert_eq!(Action::from_name("grid_toggle"), Some(Action::GridToggle));
    }
}
