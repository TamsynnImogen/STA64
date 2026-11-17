//! The modder tool shell (EDT-01): open an install and a mod, browse the ODFs
//! by root, edit and save one (loose files only, with a `.bak`), and preview
//! the model it names in the shared viewport.
//!
//! ```sh
//! cargo run -p sta_tools --features shell --bin tool_shell -- [<install>] [--mod <folder>]
//! ```

use std::path::PathBuf;

use bevy::asset::RenderAssetUsages;
use bevy::camera::Viewport;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use sta_tools::shell::{self, Workspace};

#[derive(Resource, Default)]
struct Shell {
    install: String,
    mods: Vec<String>,
    /// Index into `mods`, `None` for the plain install.
    active_mod: Option<usize>,
    ws: Option<Workspace>,
    filter: String,
    selected: Option<String>,
    text: String,
    dirty: bool,
    status: String,
    /// The ODF whose model the viewport shows.
    previewed: Option<String>,
}

/// The previewed model's entity.
#[derive(Component)]
struct Preview;

/// Orbit of the preview camera: yaw, distance.
#[derive(Component)]
struct Orbit(f32, f32);

fn main() {
    let mut args = std::env::args().skip(1);
    let mut shell = Shell::default();
    let mut wanted_mod = None;
    while let Some(a) = args.next() {
        if a == "--mod" {
            wanted_mod = args.next();
        } else {
            shell.install = a;
        }
    }
    if !shell.install.is_empty() {
        shell.mods = shell::mods(&PathBuf::from(&shell.install));
        shell.active_mod = wanted_mod.and_then(|m| shell.mods.iter().position(|x| x.eq_ignore_ascii_case(&m)));
        open(&mut shell);
    }
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: "STA64 tool shell".into(), ..default() }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .insert_resource(shell)
        .insert_resource(ClearColor(Color::srgb(0.06, 0.07, 0.1)))
        .add_systems(Startup, setup)
        .add_systems(EguiPrimaryContextPass, ui)
        .add_systems(Update, (preview, orbit))
        .run();
}

fn open(s: &mut Shell) {
    let m = s.active_mod.and_then(|i| s.mods.get(i)).cloned();
    match Workspace::open(&PathBuf::from(&s.install), m.as_deref()) {
        Ok(ws) => {
            s.status = format!("{} ODFs in {} roots", ws.odf_count(), ws.roots().len());
            s.ws = Some(ws);
        }
        Err(e) => {
            s.status = e.to_string();
            s.ws = None;
        }
    }
    s.selected = None;
    s.text.clear();
    s.dirty = false;
}

fn setup(mut commands: Commands) {
    commands.spawn((Camera3d::default(), Transform::from_xyz(0.0, 30.0, 120.0).looking_at(Vec3::ZERO, Vec3::Y), Orbit(0.0, 120.0)));
    commands.spawn((DirectionalLight { illuminance: 8000.0, ..default() }, Transform::from_xyz(50.0, 80.0, 40.0).looking_at(Vec3::ZERO, Vec3::Y)));
}

fn ui(mut contexts: EguiContexts, mut s: ResMut<Shell>, mut camera: Single<&mut Camera>, window: Single<&Window, With<PrimaryWindow>>) -> Result {
    let ctx = contexts.ctx_mut()?;
    let s = &mut *s;
    // egui 0.36 panels are laid out inside a root ui covering the window.
    let mut root = egui::Ui::new(ctx.clone(), "shell".into(), egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.viewport_rect()));
    let left = egui::Panel::left("browser")
        .resizable(true)
        .default_size(280.0)
        .show(&mut root, |ui| {
            ui.heading("Install");
            ui.text_edit_singleline(&mut s.install);
            if ui.button("Scan mods").clicked() {
                s.mods = shell::mods(&PathBuf::from(&s.install));
                s.active_mod = None;
            }
            let label = s.active_mod.and_then(|i| s.mods.get(i)).cloned().unwrap_or_else(|| "(no mod)".into());
            egui::ComboBox::from_label("Mod").selected_text(label).show_ui(ui, |ui| {
                ui.selectable_value(&mut s.active_mod, None, "(no mod)");
                for (i, m) in s.mods.iter().enumerate() {
                    ui.selectable_value(&mut s.active_mod, Some(i), m);
                }
            });
            if ui.button("Open").clicked() {
                open(s);
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Filter");
                ui.text_edit_singleline(&mut s.filter);
            });
            let filter = s.filter.to_ascii_lowercase();
            let mut pick = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                let Some(ws) = &s.ws else { return };
                for root in ws.roots() {
                    let shown: Vec<_> = root.odfs.iter().filter(|(stem, _)| filter.is_empty() || stem.contains(&filter)).collect();
                    egui::CollapsingHeader::new(format!("{} ({})", root.name, shown.len())).id_salt(&root.name).show(ui, |ui| {
                        for (stem, path) in shown {
                            if ui.selectable_label(s.selected.as_deref() == Some(stem.as_str()), stem).on_hover_text(path).clicked() {
                                pick = Some(stem.clone());
                            }
                        }
                    });
                }
            });
            if let Some(stem) = pick
                && let Some(ws) = &s.ws
            {
                match ws.read_odf(&stem) {
                    Ok(t) => {
                        s.text = t;
                        s.dirty = false;
                        s.status = ws.loose_path(&stem).map_or_else(|e| e.to_string(), |p| p.display().to_string());
                        s.selected = Some(stem);
                    }
                    Err(e) => s.status = e.to_string(),
                }
            }
        })
        .response
        .rect
        .width();
    let right = egui::Panel::right("editor")
        .resizable(true)
        .default_size(460.0)
        .show(&mut root, |ui| {
            ui.horizontal(|ui| {
                ui.heading(s.selected.clone().unwrap_or_else(|| "No ODF".into()));
                let can_save = s.dirty && s.selected.is_some();
                if ui.add_enabled(can_save, egui::Button::new("Save")).clicked()
                    && let (Some(stem), Some(ws)) = (s.selected.clone(), s.ws.as_mut())
                {
                    s.status = match ws.save(&stem, &s.text) {
                        Ok(bak) => {
                            s.dirty = false;
                            format!("saved; previous file kept as {}", bak.display())
                        }
                        Err(e) => e.to_string(),
                    };
                }
            });
            ui.label(&s.status);
            egui::ScrollArea::vertical().show(ui, |ui| {
                if ui.add(egui::TextEdit::multiline(&mut s.text).code_editor().desired_width(f32::INFINITY)).changed() {
                    s.dirty = true;
                }
            });
        })
        .response
        .rect
        .width();
    // The shared viewport: what the panels leave.
    let k = window.scale_factor();
    let (l, r) = ((left * k) as u32, (right * k) as u32);
    let (w, h) = (window.physical_width(), window.physical_height());
    if w > l + r + 1 && h > 1 {
        camera.viewport = Some(Viewport { physical_position: UVec2::new(l, 0), physical_size: UVec2::new(w - l - r, h), ..default() });
    }
    Ok(())
}

/// Rebuild the preview when the selection changes.
fn preview(
    mut commands: Commands,
    mut s: ResMut<Shell>,
    old: Query<Entity, With<Preview>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cam: Single<&mut Orbit>,
) {
    if s.previewed == s.selected {
        return;
    }
    s.previewed = s.selected.clone();
    for e in &old {
        commands.entity(e).despawn();
    }
    let (Some(stem), Some(ws)) = (s.selected.clone(), s.ws.as_ref()) else { return };
    let Some(model) = ws.model_name(&stem).and_then(|n| ws.load_sod(&n)) else { return };
    let (positions, indices) = shell::preview_triangles(&model);
    if indices.is_empty() {
        return;
    }
    // Direct3D's left-handed space to Bevy's right-handed: mirror z and swap the winding.
    let positions: Vec<[f32; 3]> = positions.iter().map(|p| [p[0], p[1], -p[2]]).collect();
    let indices: Vec<u32> = indices.chunks(3).flat_map(|t| [t[0], t[2], t[1]]).collect();
    let radius = positions.iter().map(|p| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()).fold(1.0f32, f32::max);
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_indices(Indices::U32(indices));
    mesh.duplicate_vertices();
    mesh.compute_flat_normals();
    commands.spawn((
        Preview,
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(StandardMaterial { base_color: Color::srgb(0.7, 0.72, 0.78), perceptual_roughness: 0.6, double_sided: true, cull_mode: None, ..default() })),
        Transform::default(),
    ));
    cam.1 = radius * 2.6;
}

/// Turn the preview camera slowly round the model.
fn orbit(time: Res<Time>, mut q: Query<(&mut Orbit, &mut Transform)>) {
    for (mut o, mut t) in &mut q {
        o.0 += time.delta_secs() * 0.4;
        let d = o.1;
        *t = Transform::from_xyz(d * o.0.sin(), d * 0.35, d * o.0.cos()).looking_at(Vec3::ZERO, Vec3::Y);
    }
}
