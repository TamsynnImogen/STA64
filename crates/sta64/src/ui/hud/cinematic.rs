//! The HUD's in-play movie panel (UI-39): the window a mission's `PlayCinematicMovie`
//! (Armada II) or `PlayBridgeMovie` (Armada I) plays in while the game goes on.
//!
//! * **Stock HUD** ([`PanelKind::Cinematic`]): the sheet's `cinematicPanelArea` (a screen
//!   place, so it moves with the sheet in both HUD layouts, like every other panel), the
//!   `cinematicBackgroundPanel` pieces and `cinematicDisplayArea` (panel-relative) from the
//!   install's gui cfg. A cfg with no visible `cinematicPanelArea` has no panel: nothing is
//!   registered and the movie plays full screen.
//! * **Generic HUD** ([`PanelKind::Viewscreen`]): a framed rectangle at STA64's own place
//!   ([`HudLayout::viewscreen`]), arranged and not measured. The Armada I bridge scene
//!   (backdrop, layers, the clip's rectangle inside it) is not drawn: its cfg keys give a
//!   640x480 scene and no key places the clip.
//!
//! The panel registers [`PanelScreen`] with its kind and removes it when it has no rectangle.
//! It is shown only while a movie plays in a panel, never takes the pointer (play goes on
//! around it), and the movie is fitted into the display rectangle at its own aspect.

use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::layout::{HudLayout, R, VIEWSCREEN_PAD};
use super::panels::HudRoot;
use super::stock::{self, layout::StockLayout};
use super::{HudConfig, HudLayoutRes};
use crate::media::{Movies, PanelKind, PanelScreen};
use crate::ui::gui_sprites::GuiImage;
use crate::ui::profile::UiProfile;
use crate::ui::theme::Theme;

/// Everything that decides how the panel looks and where it is, in HUD (theme) units.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Spec {
    pub kind: PanelKind,
    pub panel: R,
    /// Where the movie is drawn (inside `panel`).
    pub display: R,
    /// Background art of the stock sheet: screen rectangle, sprite name, frame.
    pub pieces: Vec<(R, String, usize)>,
    /// The generic HUD's frame colours (panel, border); `None` on the stock sheet.
    pub frame: Option<(Color, Color)>,
}

/// The stock sheet's cinematic panel, `None` when the cfg has no visible `cinematicPanelArea`.
pub(super) fn stock_spec(layout: &StockLayout) -> Option<Spec> {
    let panel = layout.panel("cinematic")?;
    let shift = |r: R| R::new(panel.x + r.x, panel.y + r.y, r.w, r.h);
    let display = layout
        .rect("cinematicDisplayArea")
        .or_else(|| layout.rect("cinematicBackgroundArea"))
        .map_or(panel, shift);
    let pieces = layout
        .pieces("cinematicBackgroundPanel")
        .into_iter()
        .map(|p| (shift(p.rect), p.name, p.frame))
        .collect();
    Some(Spec { kind: PanelKind::Cinematic, panel, display, pieces, frame: None })
}

/// The generic HUD's viewscreen: its own rectangle, a frame of [`VIEWSCREEN_PAD`] around the picture.
pub(super) fn viewscreen_spec(layout: &HudLayout, theme: &Theme) -> Spec {
    let p = layout.viewscreen;
    let display = R::new(p.x + VIEWSCREEN_PAD, p.y + VIEWSCREEN_PAD, p.w - 2.0 * VIEWSCREEN_PAD, p.h - 2.0 * VIEWSCREEN_PAD);
    Spec { kind: PanelKind::Viewscreen, panel: p, display, pieces: Vec::new(), frame: Some((theme.panel, theme.accent2)) }
}

/// The largest rectangle of the picture's aspect that fits `display`, centred in it.
pub(super) fn fit(display: R, picture: (u32, u32)) -> R {
    let (pw, ph) = (picture.0 as f32, picture.1 as f32);
    if pw <= 0.0 || ph <= 0.0 || display.w <= 0.0 || display.h <= 0.0 {
        return display;
    }
    let scale = (display.w / pw).min(display.h / ph);
    let (w, h) = ((pw * scale).round(), (ph * scale).round());
    R::new(display.x + ((display.w - w) / 2.0).round(), display.y + ((display.h - h) / 2.0).round(), w, h)
}

/// What is on screen now.
#[derive(Resource, Default)]
pub(super) struct CinematicState {
    spec: Option<Spec>,
    image: Option<Handle<Image>>,
    entity: Option<Entity>,
}

/// The container of the panel: hidden while no movie plays in a panel.
#[derive(Component)]
pub(super) struct CinematicPanel;

/// The movie's picture node and the display rectangle it is fitted into.
#[derive(Component)]
pub(super) struct CinematicPicture {
    display: R,
}

fn absolute(r: R) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(r.x),
        top: Val::Px(r.y),
        width: Val::Px(r.w),
        height: Val::Px(r.h),
        ..default()
    }
}

fn blank() -> Image {
    Image::new_fill(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    )
}

/// The panel the HUD in force has, from the layouts it already computed.
fn wanted(
    profile: Option<&UiProfile>,
    config: &HudConfig,
    stock: &stock::StockState,
    layout: &HudLayoutRes,
    theme: &Theme,
) -> Option<Spec> {
    if stock::enabled(profile, config) {
        stock.layout.as_ref().and_then(stock_spec)
    } else {
        Some(viewscreen_spec(&layout.0, theme))
    }
}

/// Keep the panel and the registered [`PanelScreen`] in step with the HUD in force.
#[allow(clippy::too_many_arguments)]
pub(super) fn build(
    mut commands: Commands,
    profile: Option<Res<UiProfile>>,
    config: Res<HudConfig>,
    stock: Res<stock::StockState>,
    layout: Res<HudLayoutRes>,
    theme: Res<Theme>,
    roots: Query<Entity, With<HudRoot>>,
    images: Option<ResMut<Assets<Image>>>,
    mut state: ResMut<CinematicState>,
) {
    let Some(mut images) = images else { return };
    let spec = wanted(profile.as_deref(), &config, &stock, &layout, &theme);
    // The panel entity is gone when the HUD root was rebuilt.
    let alive = state.entity.is_some_and(|e| commands.get_entity(e).is_ok());
    if spec == state.spec && (spec.is_none() || alive) {
        return;
    }
    if let Some(e) = state.entity.take() {
        commands.entity(e).despawn();
    }
    state.spec.clone_from(&spec);
    let Some(spec) = spec else {
        commands.remove_resource::<PanelScreen>();
        return;
    };
    let Ok(root) = roots.single() else {
        state.spec = None;
        return;
    };
    let image = state.image.get_or_insert_with(|| images.add(blank())).clone();
    commands.insert_resource(PanelScreen { image: image.clone(), kind: spec.kind });

    let container = commands
        .spawn((
            CinematicPanel,
            Pickable::IGNORE,
            Node { display: Display::None, position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
            GlobalZIndex(8),
            ChildOf(root),
        ))
        .id();
    if let Some((fill, border)) = spec.frame {
        commands.spawn((
            Node { border: UiRect::all(Val::Px(2.0)), ..absolute(spec.panel) },
            BackgroundColor(fill),
            BorderColor::all(border),
            Pickable::IGNORE,
            ChildOf(container),
        ));
    }
    commands.spawn((absolute(spec.display), BackgroundColor(Color::BLACK), Pickable::IGNORE, ChildOf(container)));
    commands.spawn((
        CinematicPicture { display: spec.display },
        absolute(spec.display),
        ImageNode::new(image),
        Pickable::IGNORE,
        ChildOf(container),
    ));
    // The sheet's frame art sits over the picture's edges, as on the original panel.
    for (rect, name, frame) in &spec.pieces {
        commands.spawn((absolute(*rect), GuiImage::new(name.clone()).with_frame(*frame), Pickable::IGNORE, ChildOf(container)));
    }
    state.entity = Some(container);
}

/// Show the panel while a movie plays in a panel; fit the picture to the display rectangle.
pub(super) fn show(
    movies: Option<Res<Movies>>,
    images: Option<Res<Assets<Image>>>,
    screen: Option<Res<PanelScreen>>,
    mut panels: Query<&mut Node, With<CinematicPanel>>,
    mut pictures: Query<(&CinematicPicture, &mut Node), Without<CinematicPanel>>,
) {
    let on = movies.zip(screen.as_ref()).is_some_and(|(m, s)| m.panel_busy_for(&s.image));
    let display = if on { Display::Flex } else { Display::None };
    for mut node in &mut panels {
        if node.display != display {
            node.display = display;
        }
    }
    let (Some(images), Some(screen)) = (images, screen) else { return };
    let Some(img) = images.get(&screen.image) else { return };
    for (picture, mut node) in &mut pictures {
        let r = fit(picture.display, (img.width(), img.height()));
        let next = absolute(r);
        if node.left != next.left || node.top != next.top || node.width != next.width || node.height != next.height {
            node.left = next.left;
            node.top = next.top;
            node.width = next.width;
            node.height = next.height;
        }
    }
}

#[cfg(test)]
mod tests {
    use sta_content::GuiCfg;

    use super::*;
    use crate::app::configure_sets;
    use crate::media::{MovieId, MovieTarget, PlayMovie};
    use crate::sim_link::{Selection};
    use crate::ui::hud::{ExternalFeed, HudFeed, HudPlugin};

    /// Authored sheet text with a cinematic panel (numbers match no stock sheet).
    const CFG: &str = "\
screenWidth = 1600
screenHeight = 1200
resourcePanelArea = 0 0 400 80
minimapPanelArea = 0 800 400 400
infoPanelArea = 400 900 500 300
cinematicPanelArea = 1000 700 400 300
cinematicBackgroundPanel = \"cine_panel\"
cinematicBackgroundPanelSize = 2
cinematicBackgroundPanel_0 = 0 0 400 20
cinematicBackgroundPanel_1 = 0 280 400 20
cinematicDisplayArea = 20 30 360 240
";

    fn layout_of(text: &str) -> GuiLayout {
        let (cfg, warnings) = GuiCfg::parse("synthetic.cfg", text);
        assert!(warnings.is_empty(), "{warnings:?}");
        cfg.layout_auto()
    }

    use sta_content::GuiLayout;

    fn stock(text: &str, screen: Vec2) -> StockLayout {
        StockLayout::new(&layout_of(text), screen)
    }

    #[test]
    fn the_stock_panel_is_the_cfg_area_in_the_sheet_with_its_display_inside() {
        // A 4:3 window of the sheet's own aspect: one canvas unit is 0.9 screen units.
        let l = stock(CFG, Vec2::new(1440.0, 1080.0));
        let spec = stock_spec(&l).expect("the cfg has a visible cinematic area");
        assert_eq!(spec.kind, PanelKind::Cinematic);
        let s = l.scale;
        assert_eq!(spec.panel, R::new((1000.0 * s).round() + l.hud.x, (700.0 * s).round() + l.hud.y, (400.0 * s).round(), (300.0 * s).round()));
        // The display area is panel-relative: moved with the panel, inside it.
        assert_eq!(spec.display.x - spec.panel.x, (20.0 * s).round());
        assert_eq!(spec.display.y - spec.panel.y, (30.0 * s).round());
        assert!(spec.display.right() <= spec.panel.right() && spec.display.bottom() <= spec.panel.bottom());
        assert_eq!(spec.pieces.len(), 2);
        assert!(spec.pieces.iter().all(|(r, name, _)| name == "cine_panel" && r.x >= spec.panel.x && r.right() <= spec.panel.right()));
        assert!(spec.frame.is_none());
    }

    #[test]
    fn the_panel_keeps_its_place_in_the_sheet_on_a_wide_window() {
        // The centred layout moves the sheet as a whole: the panel's offset from the sheet's corner is the same.
        let narrow = stock(CFG, Vec2::new(1440.0, 1080.0));
        let wide = stock(CFG, Vec2::new(2560.0, 1080.0));
        let (a, b) = (stock_spec(&narrow).unwrap(), stock_spec(&wide).unwrap());
        assert_eq!((a.panel.x - narrow.hud.x, a.panel.y - narrow.hud.y), (b.panel.x - wide.hud.x, b.panel.y - wide.hud.y));
        assert_eq!((a.panel.w, a.panel.h), (b.panel.w, b.panel.h));
        assert!(b.panel.x > a.panel.x, "the centred sheet moved right with the window");
        assert!(wide.hud.contains(Vec2::new(b.panel.x, b.panel.y)) && b.panel.right() <= wide.hud.right());
    }

    #[test]
    fn stretched_moves_the_panel_with_the_whole_sheet_and_scales_it_per_axis() {
        use super::stock::layout::HudFit;
        let screen = Vec2::new(2560.0, 1080.0);
        let l = StockLayout::with_fit(&layout_of(CFG), screen, HudFit::Stretched);
        let spec = stock_spec(&l).unwrap();
        // The sheet fills the window; the panel sits at the same fraction of it as in the cfg canvas.
        assert_eq!(l.hud, R::new(0.0, 0.0, screen.x, screen.y));
        assert!((spec.panel.x - 1000.0 / 1600.0 * screen.x).abs() <= 1.0);
        assert!((spec.panel.w - 400.0 / 1600.0 * screen.x).abs() <= 1.0);
        assert!((spec.panel.h - 300.0 / 1200.0 * screen.y).abs() <= 1.0);
        assert!(spec.panel.right() <= screen.x && spec.panel.bottom() <= screen.y);
        // The movie still keeps its own aspect inside the stretched display.
        let r = fit(spec.display, (400, 200));
        assert!((r.w / r.h - 2.0).abs() < 0.02, "{r:?}");
        // Centred on the same window: the same panel, smaller, inside the centred sheet.
        let c = stock_spec(&StockLayout::with_fit(&layout_of(CFG), screen, HudFit::Centred)).unwrap();
        assert!(c.panel.w < spec.panel.w);
    }

    #[test]
    fn no_visible_area_means_no_panel_and_the_movie_stays_fullscreen() {
        let screen = Vec2::new(1440.0, 1080.0);
        let absent = CFG.replace("cinematicPanelArea = 1000 700 400 300\n", "");
        assert!(stock_spec(&stock(&absent, screen)).is_none(), "no key");
        let zero = CFG.replace("1000 700 400 300", "-1 -1 0 0");
        assert!(stock_spec(&stock(&zero, screen)).is_none(), "an authored zero size hides it");
    }

    #[test]
    fn a_panel_without_a_display_key_shows_the_movie_in_the_whole_panel() {
        let text = CFG.replace("cinematicDisplayArea = 20 30 360 240\n", "");
        let spec = stock_spec(&stock(&text, Vec2::new(1440.0, 1080.0))).unwrap();
        assert_eq!(spec.display, spec.panel);
    }

    #[test]
    fn a_movie_is_fitted_at_its_own_aspect_and_never_stretched_to_the_display() {
        let display = R::new(100.0, 50.0, 300.0, 300.0);
        // 2:1 in a square: full width, half the height, centred.
        assert_eq!(fit(display, (400, 200)), R::new(100.0, 125.0, 300.0, 150.0));
        // Taller than the display: full height.
        assert_eq!(fit(display, (100, 200)), R::new(175.0, 50.0, 150.0, 300.0));
        // Not known yet (a 0-sized picture): the whole display.
        assert_eq!(fit(display, (0, 0)), display);
    }

    #[test]
    fn the_generic_hud_has_its_own_viewscreen_inside_the_screen_and_clear_of_the_strip() {
        let theme = Theme::default();
        let l = HudLayout::new(None, 16.0 / 9.0);
        let spec = viewscreen_spec(&l, &theme);
        assert_eq!(spec.kind, PanelKind::Viewscreen);
        assert!(spec.panel.right() <= l.screen.x && spec.panel.bottom() <= l.screen.y);
        assert!(spec.panel.y >= l.resources.bottom(), "below the resources strip");
        assert!(spec.display.x > spec.panel.x && spec.display.right() < spec.panel.right(), "framed");
        // At 4:3 as well.
        let l = HudLayout::new(None, 4.0 / 3.0);
        assert!(viewscreen_spec(&l, &theme).panel.x >= l.resources.right(), "clear of the strip at 4:3");
    }

    fn hud_app(profile: UiProfile, gui: Option<GuiLayout>) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default())).init_asset::<Image>();
        configure_sets(&mut app);
        app.insert_resource(profile)
            .insert_resource(HudConfig { gui, fit_scale: false, ..default() })
            .insert_resource(HudFeed::default())
            .insert_resource(ExternalFeed)
            .insert_resource(Selection::default())
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Movies>()
            .add_plugins(HudPlugin);
        app.update();
        app.update();
        app
    }

    fn container_display(app: &mut App) -> Option<Display> {
        app.world_mut().query_filtered::<&Node, With<CinematicPanel>>().iter(app.world()).next().map(|n| n.display)
    }

    fn queue_panel_movie(app: &mut App) {
        let image = app.world().resource::<PanelScreen>().image.clone();
        app.world_mut().resource_mut::<Movies>().queue_for_test(PlayMovie { id: MovieId(1), name: "clip".into(), target: MovieTarget::Panel(image), skippable: true });
    }

    #[test]
    fn the_stock_hud_registers_a_cinematic_panel_shown_only_while_a_movie_plays_in_a_panel() {
        let mut app = hud_app(UiProfile::Armada2, Some(layout_of(CFG)));
        assert_eq!(app.world().resource::<PanelScreen>().kind, PanelKind::Cinematic);
        assert_eq!(container_display(&mut app), Some(Display::None), "hidden with no movie");
        queue_panel_movie(&mut app);
        app.update();
        assert_eq!(container_display(&mut app), Some(Display::Flex));
        // The panel takes no pointer: nothing under the HUD's picking is blocked by it.
        let blocking = app.world_mut().query_filtered::<&Pickable, Or<(With<CinematicPanel>, With<CinematicPicture>, With<ImageNode>)>>().iter(app.world()).filter(|p| p.should_block_lower).count();
        assert_eq!(blocking, 0);
    }

    #[test]
    fn a_stock_cfg_without_a_cinematic_area_registers_nothing() {
        let text = CFG.replace("cinematicPanelArea = 1000 700 400 300\n", "");
        let mut app = hud_app(UiProfile::Armada2, Some(layout_of(&text)));
        assert!(app.world().get_resource::<PanelScreen>().is_none());
        assert_eq!(container_display(&mut app), None);
    }

    #[test]
    fn the_generic_hud_registers_the_viewscreen_kind() {
        let mut app = hud_app(UiProfile::Lcars, None);
        assert_eq!(app.world().resource::<PanelScreen>().kind, PanelKind::Viewscreen);
        assert_eq!(container_display(&mut app), Some(Display::None));
        queue_panel_movie(&mut app);
        app.update();
        assert_eq!(container_display(&mut app), Some(Display::Flex));
    }

    #[test]
    fn switching_the_hud_moves_the_registration() {
        let mut app = hud_app(UiProfile::Lcars, None);
        assert_eq!(app.world().resource::<PanelScreen>().kind, PanelKind::Viewscreen);
        *app.world_mut().resource_mut::<UiProfile>() = UiProfile::Armada2;
        app.world_mut().resource_mut::<HudConfig>().gui = Some(layout_of(CFG));
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(app.world().resource::<PanelScreen>().kind, PanelKind::Cinematic);
        let panels = app.world_mut().query_filtered::<Entity, With<CinematicPanel>>().iter(app.world()).count();
        assert_eq!(panels, 1, "the old panel was despawned");
    }

    /// Install-gated (`STA64_TEST_INSTALL`, an Armada II family root, `STA64_TEST_CFG` the race sheet,
    /// default `gui_fed.cfg`): whatever the installed sheet authors, a panel it has lies inside the
    /// scaled sheet with its display inside it, at every window aspect.
    #[test]
    fn installed_sheet_panel_lies_inside_the_sheet_at_every_aspect() {
        let Some(root) = std::env::var_os("STA64_TEST_INSTALL") else {
            eprintln!("SKIP: set STA64_TEST_INSTALL=<Armada II family root>");
            return;
        };
        let sheet = std::env::var("STA64_TEST_CFG").unwrap_or_else(|_| "gui_fed.cfg".into());
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty()).as_deref()).expect("install");
        let gui = crate::game::gui_layout(&vfs, &sheet).unwrap_or_else(|| panic!("installed {sheet} missing"));
        for (aspect, fit) in [4.0 / 3.0, 16.0 / 10.0, 16.0 / 9.0, 21.0 / 9.0].into_iter().flat_map(|a| super::stock::layout::HudFit::ALL.map(|f| (a, f))) {
            let l = StockLayout::for_aspect_fit(&gui, aspect, fit);
            let Some(spec) = stock_spec(&l) else {
                eprintln!("{sheet}: no visible cinematic area; the movie plays fullscreen");
                return;
            };
            let sheet_rect = l.hud;
            let inside = |outer: R, inner: R| inner.x >= outer.x - 0.5 && inner.y >= outer.y - 0.5 && inner.right() <= outer.right() + 0.5 && inner.bottom() <= outer.bottom() + 0.5;
            assert!(spec.panel.w > 0.0 && spec.panel.h > 0.0);
            assert!(inside(sheet_rect, spec.panel), "panel in the sheet at aspect {aspect}");
            assert!(inside(spec.panel, spec.display), "display in the panel");
        }
    }
}
