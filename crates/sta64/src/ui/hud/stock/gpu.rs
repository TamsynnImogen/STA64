//! Opt-in windowless GPU proof for installed stock CFG, SPR and texture assets.
use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::ui::{UiGlobalTransform, UiScale, UiTargetCamera};
use bevy::window::ExitCondition;
use sta_content::ContentDb;
use sta_sim::{Fixed, PlayerId, TeamId, WorldSource};
use sta_sim::setup::{GameSetup, GameOptions, PlayerSetup, Control};

use super::super::{ExternalFeed, HudConfig, HudFeed, HudPlugin, panels::HudRoot};
use super::layout::StockLayout;
use crate::app::{GameSet, configure_sets};
use crate::sim_link::Selection;
use crate::ui::gui_sprites::{GuiButton, GuiImage, GuiSkin};
use crate::ui::profile::UiProfile;
use crate::ui::radar::{RadarFeed, RadarRoot, RadarSquare};
use crate::viewer::Mounted;

#[derive(Resource)]
struct CaptureCamera(Entity);

#[allow(clippy::type_complexity)]
fn aim_roots(
    mut commands: Commands,
    camera: Res<CaptureCamera>,
    roots: Query<Entity, (Or<(With<HudRoot>, With<RadarRoot>)>, Without<UiTargetCamera>)>,
) {
    for root in &roots {
        commands.entity(root).insert(UiTargetCamera(camera.0));
    }
}

fn feed(vfs: &sta_vfs::Vfs, options: GameOptions, group: usize) -> HudFeed {
    feed_and_sim(vfs, options, group).0
}

fn feed_and_sim(vfs: &sta_vfs::Vfs, options: GameOptions, group: usize) -> (HudFeed, sta_sim::Sim) {
    // Exercise the real mounted content -> shared starting sim -> HUD path.
    // Fake ship/player values cannot expose a lost crew/officer binding.
    let (index, _) = sta_content::OdfIndex::build(vfs);
    let (db, _) = ContentDb::load(vfs, &index).expect("installed content");
    let (races, _) = sta_content::Races::load(vfs, &index);
    let race = races.get_playable("federation")
        .or_else(|| races.get_playable("unitedfederation"))
        .or_else(|| races.playable().first().copied()).expect("playable capture faction");
    let mut map = sta_content::fake::two_bases();
    map.objects.clear();
    let db = Arc::new(db);
    let setup = GameSetup {
        seed: 71, map: Arc::new(map), content: db.clone(), options,
        players: vec![PlayerSetup { id: PlayerId(0), team: TeamId(0), race: race.name.clone(), control: Control::Human }],
    };
    let (mut sim, warnings) = sta_sim::Sim::from_setup_with_starts(&setup, &races);
    eprintln!("installed HUD start warnings: {}", warnings.len());
    sim.step(&[]); // Let the existing energy system initialize its actual live pools.
    if group > 0 {
        // Authored UI fixture: FO4 starts with an outpost only. Create the requested
        // real loaded construction ships through the existing scripted edit.
        let odf = sta_content::race::basic_builder(&db, &race.name).expect("installed construction ship");
        let commands: Vec<_> = (0..group as u32).map(|seq| sta_sim::command::Command {
            player: PlayerId::SCRIPT, seq,
            kind: sta_sim::command::CommandKind::Edit(sta_sim::command::Edit::Spawn {
                odf, player: PlayerId(0), pos: sta_sim::Vec3::from_f32([(seq % 10) as f32 * 100.0,0.0,(seq / 10) as f32 * 100.0]),
                heading: (Fixed::ONE, Fixed::ZERO),
            }),
        }).collect();
        sim.step(&commands);
    }
    let view = sim.view();
    let selected = view.units.iter().find(|u| {
        u.owner == PlayerId(0) && u.flags.contains(sta_sim::view::UnitFlags::SELECTABLE)
    }).expect("selectable authored start unit").id;
    let feed = HudFeed { view, player: PlayerId(0), selected: vec![selected], content: Some(db), production: sim.production_view(), ..Default::default() };
    (feed, sim)
}

fn resolved_pieces(app: &mut App, pieces: &[super::layout::Piece]) -> usize {
    app.world_mut()
        .query::<(&GuiImage, &ImageNode)>()
        .iter(app.world())
        .filter(|(image, _)| {
            pieces.iter().any(|piece| {
                image.frame == piece.frame && image.names.iter().any(|n| n.eq_ignore_ascii_case(&piece.name))
            })
        })
        .count()
}

/// `STA64_HUD_SIZES=3440x1440,2752x1152` replaces a test's frame sizes (a local check at the
/// window a player reports); without it the test's own list.
fn sizes(default: &[UVec2]) -> Vec<UVec2> {
    let Ok(list) = std::env::var("STA64_HUD_SIZES") else { return default.to_vec() };
    let parsed: Vec<UVec2> = list
        .split(',')
        .filter_map(|s| s.trim().split_once('x').and_then(|(w, h)| Some(UVec2::new(w.parse().ok()?, h.parse().ok()?))))
        .collect();
    if parsed.is_empty() { default.to_vec() } else { parsed }
}

#[derive(Clone, Copy)]
enum HoverCapture { Selected, SelectedHover, Low, Other, BuildPage, ColonyShip, Group(usize), Avatar, WeaponIcons }

/// Independent installed-data expectation: current crew is compared with its
/// actual live capacity, not crewCost. Resolve authored palette/thresholds.
fn crew_colour(vfs: &sta_vfs::Vfs, feed: &HudFeed, unit: &sta_sim::UnitView, stock: &StockLayout) -> Color {
    if let Some(colour) = stock.colour("crew_amt_color") { return colour; }
    let [now, max] = unit.crew.expect("visible crew pool");
    let ratio = if max == Fixed::ZERO { 1.0 } else { (now.to_bits() as f64 / max.to_bits() as f64).min(1.0) };
    let tier = if ratio <= 0.0 { "systemIconDestroyedColor" }
        else if ratio > 0.5 { "systemIconHealthyColor" }
        else if ratio > 0.25 { "systemIconLowColor" }
        else { "systemIconCriticalColor" };
    if let Some(colour) = stock.colour(tier) { return colour; }
    let (index, _) = sta_content::OdfIndex::build(vfs);
    let (races, _) = sta_content::Races::load(vfs, &index);
    let race = feed.content.as_ref().and_then(|db| db.unit(unit.def)).and_then(|def| races.get(&def.race));
    let Some(race) = race else { return Color::srgb(0.0, 1.0, 0.0) };
    let odf = sta_assets::a2fmt::odf::Odf::load_layered(&format!("{}.odf",race.stem), |name, version| index.load(vfs,name,version).map(String::into_bytes)).unwrap();
    let yellow = odf.get_f32("crewYellowStatus").filter(|v| v.is_finite()).unwrap_or(0.5) as f64;
    let red = odf.get_f32("crewRedStatus").filter(|v| v.is_finite()).unwrap_or(0.25) as f64;
    if ratio > yellow { Color::srgb(0.0, 1.0, 0.0) }
    else if ratio > red { Color::srgb(1.0, 1.0, 0.0) }
    else { Color::srgb(1.0, 0.0, 0.0) }
}

/// `STA64_HUD_FIT=stretched` captures the player's stretched HUD layout; anything else, centred.
fn env_fit() -> super::layout::HudFit {
    std::env::var("STA64_HUD_FIT").ok().and_then(|v| super::layout::HudFit::from_key(&v)).unwrap_or_default()
}

fn capture_one(vfs: sta_vfs::Vfs, profile: UiProfile, size: UVec2, cfg: &str, label: &str, options: GameOptions, hover_mode: HoverCapture) {
    let layout = crate::game::gui_layout(&vfs, cfg).unwrap_or_else(|| panic!("installed {cfg} missing"));
    let stock = StockLayout::for_aspect_fit(&layout, size.x as f32 / size.y as f32, env_fit());
    let resource = stock.pieces("resourcePanel");
    let minimap = stock.pieces("minimapBackgroundPanel");
    let info = stock.pieces("infoMiddleBackgroundPanel");
    assert!(
        !resource.is_empty() || !minimap.is_empty() || !info.is_empty(),
        "{label}: CFG has no authored background pieces to check"
    );
    let group_count = if let HoverCapture::Group(count) = hover_mode { count } else { 0 };
    let mut hud_feed = feed(&vfs, options, group_count);
    let original = hud_feed.selected[0];
    let hover = match hover_mode {
        HoverCapture::Selected | HoverCapture::BuildPage => None,
        HoverCapture::SelectedHover => Some(original),
        HoverCapture::Group(count) => {
            hud_feed.selected = hud_feed.view.units.iter().rev().filter(|u| u.owner == hud_feed.player
                && hud_feed.content.as_ref().unwrap().unit(u.def).is_some_and(|d| d.class.0 == "constructionrig"))
                .take(count).map(|u| u.id).collect();
            assert_eq!(hud_feed.selected.len(), count, "scripted construction ships for tier capture");
            None
        }
        HoverCapture::Low => { hud_feed.selected.clear(); Some(original) },
        HoverCapture::Avatar => { hud_feed.selected.clear(); None },
        HoverCapture::WeaponIcons => {
            let def = hud_feed.content.as_ref().unwrap().key("fed_intrepidy").expect("FO4 Intrepid");
            let mut view = (*hud_feed.view).clone();
            view.units.iter_mut().find(|u| u.id == original).unwrap().def = def;
            hud_feed.view = Arc::new(view);
            None
        }
        // UI-38: the start unit's view row (same id, so it stays visible) with a colony ship's definition.
        HoverCapture::ColonyShip => {
            let Some(def) = hud_feed.content.as_ref().unwrap().key("fcolony") else {
                eprintln!("SKIP {label}: the mounted install has no `fcolony` definition");
                return;
            };
            let mut view = (*hud_feed.view).clone();
            view.units.iter_mut().find(|u| u.id == original).unwrap().def = def;
            hud_feed.view = Arc::new(view);
            None
        }
        HoverCapture::Other => Some(hud_feed.view.units.iter().find(|unit| {
            unit.id != original && unit.def != hud_feed.view.unit(original).unwrap().def
                && unit.flags.contains(sta_sim::view::UnitFlags::SELECTABLE)
        }).expect("a distinct authored start actor for hover precedence").id),
    };
    let source = if matches!(hover_mode, HoverCapture::Group(_)) { None } else { hover.or_else(|| hud_feed.selected.first().copied()).map(|id| hud_feed.view.unit(id).unwrap()) };
    let crew_tint = source.filter(|unit| unit.crew.is_some()).map(|unit| {
        eprintln!("{label}: live unit crew current/capacity={:?}", unit.crew);
        crew_colour(&vfs, &hud_feed, unit, &stock)
    });
    if std::env::var("STA64_VERIFY_CLASSIC_CREW").as_deref() == Ok("1") && matches!(hover_mode, HoverCapture::Other) {
        let unit = source.expect("Classic starbase hover fixture");
        assert_eq!(hud_feed.content.as_ref().unwrap().unit(unit.def).unwrap().odf, "fbase");
        assert_eq!(unit.crew, Some([Fixed::from_int(150), Fixed::from_int(1500)]));
        assert_eq!(crew_tint, Some(Color::srgb(1.0, 0.0, 0.0)), "Classic starbase at10percent must be red");
    }
    let percent = |now: i128, max: i128| (max > 0).then(|| (now * 200 + max).div_euclid(max * 2).to_string());
    let whole = |value: Fixed| { let bits=value.to_bits(); if bits>0 { (bits/Fixed::ONE.to_bits()).max(1) } else { bits/Fixed::ONE.to_bits() } };
    let expected_bar = {
        use super::infobar::InfoBarField::{Name,Hull,Shields,Crew,Energy};
        // The InfoBar is driven by hover. Selection alone fills the separate info panel.
        let hovered = hover.map(|id| hud_feed.view.unit(id).expect("hovered fixture unit"));
        let expected = hovered.map(|unit| {
            let shield = unit.shields.iter().fold([0_i128;2], |[n,m],s| [n+i128::from(s[0].to_bits()),m+i128::from(s[1].to_bits())]);
            [
                Some(hud_feed.unit_label(unit)),
                percent(i128::from(unit.hull[0].to_bits()),i128::from(unit.hull[1].to_bits())),
                percent(shield[0],shield[1]),
                unit.crew.map(|v| whole(v[0]).to_string()),
                unit.energy.and_then(|v| percent(i128::from(v[0].to_bits()),i128::from(v[1].to_bits()))),
            ]
        });
        let fields = [
            (Name,"shipName"), (Hull,"hull"), (Shields,"shields"),
            (Crew,"crew_amt"), (Energy,"energy_amt"),
        ];
        fields.into_iter().zip(expected.unwrap_or_default())
            .map(|((field, key), value)| (field, key, value))
            .collect::<Vec<_>>()
    };
    let bar_variant = super::layout::info_variant(&super::super::model::info(&hud_feed));
    let has_info_bar = stock.info_area(bar_variant).is_some();
    let info = stock.pieces(match bar_variant {
        0 => "infoLowBackgroundPanel",
        1 => "infoMiddleBackgroundPanel",
        _ => "infoBackgroundPanel",
    });
    let player = hud_feed.view.player(hud_feed.player).expect("capture player");
    let whole = |value: Fixed| value.to_bits() / Fixed::ONE.to_bits();
    let crew_text = player.crew.map(|v| whole(v).to_string()).unwrap_or_default();
    let officer_text = format!("{}/{}", whole(player.officers[0]), whole(player.officers[1]));
    let selection = hud_feed.selected.clone();
    let radar_feed = RadarFeed {
        view: hud_feed.view.clone(),
        player: hud_feed.player,
        selected: hud_feed.selected.clone(),
        content: hud_feed.content.clone(),
        camera: None,
    };
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .disable::<bevy::gilrs::GilrsPlugin>()
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    configure_sets(&mut app);
    app.insert_resource(ClearColor(Color::BLACK))
        .insert_resource(profile)
        .insert_resource(crate::input::hover::HoveredUnit(hover))
        .insert_resource(profile.theme())
        .insert_resource(Mounted(vfs))
        .insert_resource(GuiSkin {
            table: Some("gui_federation.spr".into()),
        })
        .insert_resource(HudConfig {
            gui: Some(layout),
            aspect: size.x as f32 / size.y as f32,
            fit: env_fit(),
            fit_scale: false,
            log: false,
        })
        .insert_resource(UiScale(size.y as f32 / 1080.0))
        .insert_resource(hud_feed)
        .insert_resource(radar_feed)
        .insert_resource(ExternalFeed)
        .insert_resource(Selection(selection))
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(HudPlugin)
        .add_systems(Update, aim_roots.before(GameSet::Ui));

    let mut image = Image::new_target_texture(
        size.x,
        size.y,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        None,
    );
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let camera = app
        .world_mut()
        .spawn((
            Camera2d,
            Camera {
                order: 1,
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            RenderTarget::from(target.clone()),
            Msaa::Off,
        ))
        .id();
    app.insert_resource(CaptureCamera(camera));
    if matches!(hover_mode, HoverCapture::Avatar) {
        app.insert_resource(crate::render::units::LocalPlayer(Some(PlayerId(0))));
    }
    if matches!(hover_mode, HoverCapture::BuildPage) {
        // The builder's build page: command-button labels and PREV/NEXT when it has pages.
        app.world_mut().resource_mut::<super::super::HudState>().page = super::super::Page::Build;
    }
    app.finish();
    app.cleanup();
    for _ in 0..16 {
        app.update();
    }

    let adapter = app
        .world()
        .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        .cloned()
        .or_else(|| {
            app.get_sub_app(bevy::render::RenderApp).and_then(|r| {
                r.world()
                    .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
                    .cloned()
            })
        });
    let Some(adapter) = adapter else {
        eprintln!("SKIP {label}: no render adapter");
        return;
    };
    eprintln!("{label} GPU adapter: {}", adapter.name);
    if !matches!(hover_mode, HoverCapture::Group(_)) && crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
        eprintln!("SKIP {label}: hardware adapter unavailable");
        return;
    }

    if profile == UiProfile::FleetOps
        && std::env::var("STA64_TEST_MOD").as_deref() == Ok("FO4")
        && matches!(hover_mode, HoverCapture::Selected | HoverCapture::SelectedHover)
    {
        let count = app.world_mut().query::<(&GuiImage, &ImageNode)>().iter(app.world())
            .filter(|(sprite, image)| sprite.names.iter().any(|name| name == "fed_outpost_si") && image.rect.is_some())
            .count();
        assert_eq!(count, 1, "{label}: FO4 outpost systems atlas entry must resolve");
    }

    if matches!(hover_mode, HoverCapture::Avatar) {
        let mut query = app.world_mut().query_filtered::<(&GuiButton, &ImageNode, &ComputedNode, &UiGlobalTransform), With<super::super::avatar::Portrait>>();
        let mut choices: Vec<_> = query.iter(app.world()).collect();
        choices.sort_by(|a, b| a.3.translation.x.total_cmp(&b.3.translation.x));
        assert_eq!(choices.len(), 2, "both full-size avatar portraits resolve");
        assert_eq!(choices[0].0.names, ["b_avatar_fed_mayson"]);
        assert_eq!(choices[1].0.names, ["b_avatar_fed_risner"]);
        let panel = stock.panel("CommandInfoBar").expect("FO4 choice panel");
        let first = stock.at(Vec2::new(panel.x, panel.y), "CommandInfoBarButtonRect").expect("FO4 portrait rect");
        let stride = stock.number("CommandInfoBarButtonDistance").unwrap() as f32 * stock.scale_x;
        for (_, image, node, _) in &choices {
            assert_eq!(image.rect.unwrap().size(), Vec2::splat(256.0));
            assert_eq!(node.size, Vec2::new(first.w, first.h));
        }
        let first_center = Vec2::new(first.x + first.w / 2.0, first.y + first.h / 2.0);
        assert!(choices[0].3.translation.distance(first_center) < 1.0);
        assert!(choices[1].3.translation.distance(first_center + Vec2::new(stride, 0.0)) < 1.0);
        let center_x = (choices[0].3.translation.x + choices[1].3.translation.x) * 0.5;
        assert!((center_x - size.x as f32 / 2.0).abs() < 1.0);
    }
    if matches!(hover_mode, HoverCapture::WeaponIcons) {
        let mut query = app.world_mut().query::<(&super::weapon_icons::WeaponIcon, &GuiImage, &ImageNode, &Visibility)>();
        let mut slots: Vec<_> = query.iter(app.world()).map(|(icon, name, image, visibility)| {
            assert!(image.rect.is_some());
            assert_eq!(*visibility, Visibility::Inherited);
            assert_eq!(name.names.last().unwrap(), "systemicon_default");
            if icon.slot == 1 {
                assert_eq!(image.color, super::weapon_icons::READY);
            }
            icon.slot
        }).collect();
        slots.sort_unstable();
        assert_eq!(slots, [1, 2, 3, 4], "all mounted Intrepid weapon icons resolve");
    }

    if let HoverCapture::Group(count) = hover_mode {
        let ids = app.world().resource::<HudFeed>().selected.clone();
        let rendered: Vec<_> = app.world_mut().query::<(Entity, &super::super::panels::GroupTile)>()
            .iter(app.world()).map(|(entity,tile)| (entity,tile.0)).collect();
        assert_eq!(rendered.len(), count, "{label}: all requested cfg slots render");
        for (index, id) in ids.iter().enumerate() {
            let entity = rendered.iter().find(|(_, tile_id)| tile_id == id).unwrap().0;
            let name = app.world().resource::<HudFeed>().name(app.world().resource::<HudFeed>().view.unit(*id).unwrap());
            let mut texts = app.world_mut().query::<(&crate::ui::sprite_font::SpriteText, &Node, &crate::ui::sprite_font::SpriteTextStyle, &ChildOf)>();
            let labels: Vec<_> = texts.iter(app.world()).filter(|(_,_,_,parent)|
                app.world().get::<ChildOf>(parent.parent()).is_some_and(|p| p.parent() == entity)).collect();
            if count >= 17 {
                assert!(labels.is_empty(), "{label}: compact tile {index} must not show its ship name");
                assert_eq!(app.world().get::<crate::ui::notices::Tooltip>(entity).unwrap().short, name);
                continue;
            }
            assert_eq!(labels.len(), 1, "{label}: one normal caption in slot {index}");
            let (text,node,style,_) = labels[0];
            assert_eq!(text.text,name);
            let origin = stock.info_origin(1);
            let rect = stock.at(origin, &format!("infoMultiShipName_{index}")).expect("installed group name area");
            let hull = stock.at(origin, &format!("infoMultiShipHull_{index}")).expect("installed hull bar");
            let (Val::Px(width), Val::Px(height)) = (node.width, node.height) else { panic!("unresolved group caption") };
            assert!(!style.native && !style.min_size && style.scaled, "name must use compact sprite-font sizing");
            if count == 2 { assert!(height <= profile.theme().micro_size + 1.0, "{label}: name {name:?} wraps or exceeds its compact size: {height}"); }
            assert!(width <= rect.w + 1.0, "{label}: name {name:?} wider than CFG area: {width} > {}", rect.w);
            assert!(height <= hull.y - rect.y + 1.0, "{label}: name {name:?} reaches hull: {height} > {}", hull.y - rect.y);
            eprintln!("{label}: {name:?} resolved {width}x{height}; name width {}, space to hull {}", rect.w, hull.y - rect.y);
        }
    }

    // Every placed stock text sits in its rectangle by the rule (the line box centred or left by
    // the rectangle's size minus one, whole-pixel origin, a taller box overflowing evenly), and is
    // neither wrapped nor clipped. Labels outside the rule (STA64's fallbacks) are listed with
    // their visibility only.
    {
        use crate::ui::sprite_font::{SpriteText, SpriteTextStyle};
        use crate::ui::stock_font::{VAlign, HAlign};
        let window_scale = app.world_mut().query::<&Window>().iter(app.world()).next().map_or(1.0, |w| w.scale_factor());
        let ui_px = app.world().get_resource::<UiScale>().map_or(1.0, |u| u.0) * window_scale;
        let on_path = app.world().get_resource::<crate::ui::sprite_font::SpriteFonts>().is_some_and(|f| f.stock_family);
        let mut texts = app.world_mut().query::<(&SpriteText, &SpriteTextStyle, &ComputedNode, &bevy::ui::UiGlobalTransform, &ChildOf, &InheritedVisibility)>();
        let (mut checked, mut wrong) = (0, Vec::new());
        for (text, style, node, at, parent, shown) in texts.iter(app.world()) {
            let slot = app.world().get::<ComputedNode>(parent.parent());
            let slot_at = app.world().get::<bevy::ui::UiGlobalTransform>(parent.parent());
            let (Some(slot), Some(slot_at)) = (slot, slot_at) else { continue };
            let (own, room) = (node.size(), slot.size());
            if own.x <= 0.0 {
                continue;
            }
            let rel = (at.translation - own / 2.0) - (slot_at.translation - room / 2.0);
            let Some(place) = style.place.filter(|p| on_path && p.rect.x > 0.0 && style.stock.is_some()) else {
                if style.stock.is_none() {
                    eprintln!("{label}: FALLBACK {:?} {own} in {room} visible={}", text.text, shown.get());
                }
                continue;
            };
            checked += 1;
            let want = Vec2::new(
                match place.h { HAlign::Left => 0.0, HAlign::Centre => ((place.rect.x - 1.0) * ui_px - own.x) / 2.0 },
                match place.v { VAlign::Top => 0.0, VAlign::Centre => ((place.rect.y - 1.0) * ui_px - own.y) / 2.0, VAlign::Bottom => (place.rect.y - 1.0) * ui_px - own.y },
            );
            eprintln!("{label}: PLACE {:?} box {own} in slot {room}: top {:.1} (rule {:.1}), left {:.1} (rule {:.1})", text.text, rel.y, want.y, rel.x, want.x);
            if (rel - want).abs().max_element() > 1.5 || style.wrap.is_some() || style.wrap_parent {
                wrong.push(format!("{:?} at {rel} (rule {want}), wraps: {}", text.text, style.wrap.is_some() || style.wrap_parent));
            }
        }
        eprintln!("{label}: {checked} placed stock text nodes checked against the placement rule");
        assert!(wrong.is_empty(), "{label}: stock text not placed by the rule: {wrong:?}");
    }

    let roots = app
        .world_mut()
        .query_filtered::<&UiTargetCamera, With<HudRoot>>()
        .iter(app.world())
        .map(|target| target.0)
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 1, "{label}: one HUD root");
    assert_eq!(roots[0], camera, "{label}: HUD targets capture camera");
    let radar_roots = app
        .world_mut()
        .query_filtered::<&UiTargetCamera, With<RadarRoot>>()
        .iter(app.world())
        .map(|target| target.0)
        .collect::<Vec<_>>();
    assert_eq!(radar_roots.len(), 1, "{label}: one radar root");
    assert_eq!(radar_roots[0], camera, "{label}: radar targets capture camera");
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<RadarSquare>>()
            .iter(app.world())
            .count(),
        1
    );
    for (name, pieces) in [("resource", &resource), ("minimap", &minimap), ("info", &info)] {
        if !pieces.is_empty() {
            assert!(
                resolved_pieces(&mut app, pieces) > 0,
                "{label}: no {name} atlas art resolved"
            );
        }
    }
    // The persistent menu and objectives buttons are outside the command palette.
    let commands = app
        .world_mut()
        .query_filtered::<(&GuiButton, &ImageNode), With<super::super::GridCell>>()
        .iter(app.world())
        .count();
    if matches!(hover_mode, HoverCapture::ColonyShip) {
        // The palette offers COLONIZE (command ODF `colonize.odf`: art `b_colonize`) for the colony ship (read from data).
        let colonize = app.world_mut().query::<(&super::super::GridCell, &GuiButton)>().iter(app.world())
            .filter(|(cell, button)| cell.0 == super::super::Act::Target(super::super::model::TargetMode::Colonize)
                && button.names.iter().any(|n| n.eq_ignore_ascii_case("b_colonize")))
            .count();
        assert_eq!(colonize, 1, "{label}: one COLONIZE button with the stock art name");
    }
    if matches!(hover_mode, HoverCapture::Low | HoverCapture::Avatar) {
        assert_eq!(commands, 0, "{label}: selection commands require a selection");
    } else {
        assert!(commands > 0, "{label}: no command button art resolved");
    }
    let resources = app
        .world_mut()
        .query::<&super::info::StockText>()
        .iter(app.world())
        .filter(|text| matches!(text, super::info::StockText::Resource(_)))
        .count();
    assert!(resources > 0, "{label}: no live resource text");
    for (id, flag, expected) in [(0, "noCrew", crew_text), (1, "noOfficers", officer_text)] {
        let visible = stock.number(flag).is_none_or(|v| v == 0.0) && stock.rect(&format!("resource_{id}")).is_some();
        let actual = app.world_mut().query::<(&super::info::StockText, &crate::ui::sprite_font::SpriteText)>()
            .iter(app.world()).find_map(|(kind,text)| matches!(kind, super::info::StockText::Resource(n) if *n == id).then(|| text.text.clone()));
        assert_eq!(actual.as_deref(), visible.then_some(expected.as_str()), "{label} CFG personnel resource_{id}");
        eprintln!("{label}: resource_{id} visible={visible} expected={expected} actual={actual:?}");
    }
    // Check the final text component too: source-label tests alone cannot
    // catch a tint lost on the way to Bevy's actual renderer.
    let defaults = [
        Color::srgb(0.0, 1.0, 0.0), Color::srgb(1.0, 1.0, 1.0),
        Color::srgb(0.6, 0.9, 1.0), Color::srgb(1.0, 1.0, 0.0),
        Color::srgb(0.8, 0.8, 0.8), Color::srgb(1.0, 0.0, 0.0),
    ];
    for (kind, text, children) in app.world_mut()
        .query::<(&super::info::StockText, &crate::ui::sprite_font::SpriteText, &Children)>()
        .iter(app.world())
    {
        let super::info::StockText::Resource(id) = *kind else { continue };
        let expected = stock.colour(&format!("resource_{id}_color")).unwrap_or(defaults[id]);
        assert_eq!(text.color, expected, "{label}: resource_{id} CFG/default tint");
        // Sprite text resolves to glyph images (the TTF fallback to Text); each carries the tint.
        let glyphs = children.iter().filter_map(|child| app.world().get::<ImageNode>(child)).collect::<Vec<_>>();
        assert!(!glyphs.is_empty(), "{label}: resource_{id} has no resolved sprite glyphs");
        assert!(glyphs.iter().all(|g| g.color == expected), "{label}: resource_{id} lost its rendered tint");
        eprintln!("{label}: resource_{id} rendered colour={expected:?}");
    }

    for (field,key,expected) in &expected_bar {
        let actual = app.world_mut().query::<(&super::infobar::InfoBarField,&crate::ui::sprite_font::SpriteText,&Visibility,Option<&Children>)>()
            .iter(app.world()).find(|(kind,_,_,_)| **kind==*field)
            .map(|(_,text,visibility,children)| (text.text.clone(),text.color,*visibility,children.map(|c| c.iter().collect::<Vec<_>>()).unwrap_or_default()));
        if !has_info_bar || stock.rect(key).is_none() {
            assert!(actual.is_none(),"{label}: disabled InfoBar {key}");
            continue;
        }
        let (text,colour,visibility,children)=actual.unwrap_or_else(|| panic!("{label}: authored InfoBar text slot {key}"));
        assert_eq!(text,expected.clone().unwrap_or_default(),"{label}: hovered InfoBar {key}");
        assert_eq!(visibility,if expected.is_some() {Visibility::Inherited} else {Visibility::Hidden});
        if expected.is_some() {
            let expected_colour=if *field==super::infobar::InfoBarField::Name {
                stock.colour("shipNameColor").unwrap_or_else(|| profile.theme().text)
            } else if *field==super::infobar::InfoBarField::Crew {crew_tint.unwrap()}
            else {stock.colour(&format!("{key}_color")).or_else(|| stock.colour(if *field==super::infobar::InfoBarField::Energy {"specialEnergyIconColor"} else {"systemIconHealthyColor"})).unwrap_or(Color::srgb(0.0,1.0,0.0))};
            assert_eq!(colour,expected_colour,"{label}: InfoBar {key} tint");
            assert!(children.iter().any(|&child| app.world().get::<ImageNode>(child).is_some_and(|g| g.color==expected_colour)),"{label}: resolved InfoBar sprite glyph tint");
        }
        eprintln!("{label}: InfoBar {key}={text:?} colour={colour:?} visibility={visibility:?}");
    }
    for (entity,kind,image,visibility) in app.world_mut().query::<(Entity,&super::infobar::InfoBarIcon,&GuiImage,&Visibility)>().iter(app.world()) {
        if matches!(hover_mode, HoverCapture::Group(_)) { continue; } // Single-actor expectations above do not describe a group.
        let (_,key,expected)=expected_bar.iter().find(|(field,_,_)|*field==kind.0).unwrap();
        assert_eq!(*visibility,if expected.is_some() {Visibility::Inherited} else {Visibility::Hidden});
        if expected.is_some() {
            assert_eq!(image.names,[*key]);
            let tint = if kind.0==super::infobar::InfoBarField::Crew {crew_tint.unwrap()}
                else {stock.colour(&format!("{key}_color")).or_else(|| stock.colour(if kind.0==super::infobar::InfoBarField::Energy {"specialEnergyIconColor"} else {"systemIconHealthyColor"})).unwrap_or(Color::srgb(0.0,1.0,0.0))};
            assert_eq!(image.color,tint);
            assert!(app.world().get::<ImageNode>(entity).is_some(),"{label}: resolved InfoBar icon {key}");
        }
    }

    let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    let output = Arc::clone(&frames);
    app.world_mut()
        .spawn(Screenshot::image(target))
        .observe(move |shot: On<ScreenshotCaptured>| {
            output.lock().unwrap().push(shot.image.clone());
        });
    for _ in 0..60 {
        app.update();
        if !frames.lock().unwrap().is_empty() {
            break;
        }
    }
    let shot = frames
        .lock()
        .unwrap()
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("{label}: no GPU capture"));
    let dynamic = shot.try_into_dynamic().expect("GPU image conversion");
    let rgba = dynamic.to_rgba8();
    if has_info_bar && hover.is_some() && crew_tint == Some(Color::srgb(1.0, 0.0, 0.0)) {
        let origin=stock.info_origin(bar_variant)+stock.point(&format!("infoBarOffset_{bar_variant}")).unwrap_or(Vec2::ZERO);
        let scale=size.y as f32/1080.0;
        for key in ["crew_amt", "icon_crew_amt"] {
            if let Some(rect)=stock.at(origin,key) {
                let mut red=0;
                for y in (rect.y*scale).max(0.0) as u32..((rect.y+rect.h)*scale).min(size.y as f32) as u32 {
                    for x in (rect.x*scale).max(0.0) as u32..((rect.x+rect.w)*scale).min(size.x as f32) as u32 {
                        let p=rgba.get_pixel(x,y);
                        if p[0]>150 && p[1]<80 && p[2]<80 {red+=1;}
                    }
                }
                assert!(red>=2,"{label}: {key} is not visibly red for low current/capacity");
                eprintln!("{label}: red {key} pixels={red}");
            }
        }
    }
    if let Ok(dir) = std::env::var("STA64_HUD_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        dynamic
            .save(std::path::Path::new(&dir).join(format!("stock-{label}{}.png", if env_fit() == super::layout::HudFit::Stretched { "-stretched" } else { "" })))
            .unwrap();
    }
    // Classic has no colour overrides. Guard the visible green/cyan/yellow
    // numerals in the real framebuffer, rather than just checking ECS state.
    if profile == UiProfile::Armada2 {
        let area = stock.panel("resource").unwrap();
        let pixel_scale = size.y as f32 / 1080.0;
        for id in [0, 2, 3] {
            let rect = stock.at(Vec2::new(area.x, area.y), &format!("resource_{id}")).unwrap();
            let rgb = stock.colour(&format!("resource_{id}_color")).unwrap_or(defaults[id]).to_srgba();
            let expected = [rgb.red, rgb.green, rgb.blue].map(|v| (v * 255.0).round() as i16);
            // Small five-digit glyphs can have no fully covered pixels.
            // Cyan needs a tighter tolerance to distinguish white text on
            // its blue panel. These bounds also reject the old white captures.
            let tolerance = if id == 2 { 24 } else { 64 };
            let mut matching = 0;
            for y in (rect.y * pixel_scale).max(0.0) as u32..((rect.y + rect.h) * pixel_scale).min(size.y as f32) as u32 {
                for x in (rect.x * pixel_scale).max(0.0) as u32..((rect.x + rect.w) * pixel_scale).min(size.x as f32) as u32 {
                    let pixel = rgba.get_pixel(x, y);
                    if (0..3).all(|channel| (pixel[channel] as i16 - expected[channel]).abs() <= tolerance) {
                        matching += 1;
                    }
                }
            }
            assert!(matching >= 2, "{label}: resource_{id} has no visible {expected:?} text pixels");
            eprintln!("{label}: resource_{id} colour-matched text pixels={matching}");
        }
    }
    let shows_hull = expected_bar.iter().any(|(field, _, value)| {
        *field == super::infobar::InfoBarField::Hull && value.is_some()
    });
    if has_info_bar && shows_hull && stock.colour("hull_color").is_none() && stock.colour("systemIconHealthyColor").is_none() {
        let origin=stock.info_origin(bar_variant)+stock.point(&format!("infoBarOffset_{bar_variant}")).unwrap_or(Vec2::ZERO);
        if let Some(rect)=stock.at(origin,"icon_hull") {
            let scale=size.y as f32/1080.0;
            let mut green=0;
            for y in (rect.y*scale).max(0.0) as u32..((rect.y+rect.h)*scale).min(size.y as f32) as u32 {
                for x in (rect.x*scale).max(0.0) as u32..((rect.x+rect.w)*scale).min(size.x as f32) as u32 {
                    let p=rgba.get_pixel(x,y);
                    if p[1]>150 && p[0]<80 && p[2]<80 {green+=1;}
                }
            }
            assert!(green>=2,"{label}: native hull icon is not visibly status-tinted");
            eprintln!("{label}: green InfoBar hull-icon pixels={green}");
        }
    }
    // A2's first two resource atlas pieces meet at canvas x=256. On a wide
    // viewport Auto letterboxes both pieces and leaves a black gap here; native
    // drawing (one factor, the sheet centred) must fill this authored join.
    if profile == UiProfile::Armada2 && size.x as f32 / size.y as f32 > 1.5 {
        let k = size.y as f32 / 1080.0;
        let x = ((stock.hud.x + 256.0 * stock.scale_x) * k).round() as u32;
        let y = ((stock.hud.y + 38.0 * stock.scale) * k).round() as u32;
        for x in x.saturating_sub(3)..=(x + 3).min(size.x - 1) {
            let pixel = rgba.get_pixel(x, y.min(size.y - 1));
            assert!(
                pixel[0].max(pixel[1]).max(pixel[2]) > 20,
                "{label}: black gap at resource atlas join ({x},{y})"
            );
        }
    }
    let coloured = rgba.pixels().filter(|p| p[0] > 20 || p[1] > 20 || p[2] > 20).count();
    eprintln!("{label}: {commands} command images, {resources} resource fields, {coloured} nonblack pixels");
    assert!(coloured > 500, "{label}: HUD did not visibly render");
}

#[test]
fn installed_stock_hud_renders_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") || std::env::var("STA64_HUD_TESTS").as_deref() != Ok("1")
    {
        eprintln!(
            "SKIP: set STA64_GPU_TESTS=1 STA64_HUD_TESTS=1 STA64_TEST_INSTALL STA64_HUD_PROFILE=a2|fo and optional STA64_TEST_MOD"
        );
        return;
    }
    let install = std::env::var("STA64_TEST_INSTALL").expect("STA64_TEST_INSTALL");
    let active_mod = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let profile = match std::env::var("STA64_HUD_PROFILE").as_deref() {
        Ok("a2") => UiProfile::Armada2,
        Ok("fo") => UiProfile::FleetOps,
        _ => panic!("set STA64_HUD_PROFILE=a2 or fo for the mounted install"),
    };
    // 21:9 and 32:9 windows (UI-37): the sheet is scaled by one factor and centred.
    let mut list = vec![
        (UVec2::new(1024, 768), "4x3".to_owned()),
        (UVec2::new(1280, 720), "16x9".to_owned()),
        (UVec2::new(1920, 1080), "16x9-1080".to_owned()),
        (UVec2::new(2560, 1080), "21x9".to_owned()),
        (UVec2::new(3840, 1080), "32x9".to_owned()),
        (UVec2::new(2752, 1152), "2752x1152".to_owned()),
    ];
    // `STA64_HUD_SIZES` replaces the list (the wide windows take the 16:9 sheet for Fleet Operations).
    if std::env::var("STA64_HUD_SIZES").is_ok() {
        list = sizes(&[]).into_iter().map(|s| (s, format!("{}x{}", s.x, s.y))).collect();
    }
    for (size, ratio) in list {
        let wide = size.x * 9 >= size.y * 16;
        let cfg = match (profile, ratio.as_str()) {
            (UiProfile::Armada2, _) => "gui_fed.cfg",
            (UiProfile::FleetOps, _) if std::env::var("STA64_HUD_SIZES").is_ok() => if wide { "gui_federation16x9.cfg" } else { "gui_federation.cfg" },
            (UiProfile::FleetOps, "16x9" | "16x9-1080" | "21x9" | "32x9" | "2752x1152") => "gui_federation16x9.cfg",
            (UiProfile::FleetOps, _) => "gui_federation.cfg",
            _ => unreachable!(),
        };
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref())
            .expect("installed stock VFS");
        let mode = if profile == UiProfile::FleetOps { "fo4" } else { "a2" };
        capture_one(vfs, profile, size, cfg, &format!("{mode}-{ratio}"), GameOptions::default(), HoverCapture::Selected);
    }
    {
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref()).unwrap();
        let (cfg, mode) = if profile == UiProfile::FleetOps { ("gui_federation16x9.cfg", "fo4") } else { ("gui_fed.cfg", "a2") };
        capture_one(vfs, profile, UVec2::new(1920, 1080), cfg, &format!("{mode}-build-page-1080"), GameOptions::default(), HoverCapture::BuildPage);
    }
    if profile == UiProfile::Armada2 {
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref()).unwrap();
        let mut options = GameOptions::default();
        options.starting_resources = sta_content::StartingResources::Lots;
        options.officer_limit = 1;
        capture_one(vfs, profile, UVec2::new(1280,720), "gui_fed.cfg", "a2-lots-restricted-16x9", options, HoverCapture::Selected);
        // UI-38: a colony ship selected (the stock command palette shows COLONIZE).
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref()).unwrap();
        capture_one(vfs, profile, UVec2::new(1024, 768), "gui_fed.cfg", "a2-colony-ship-4x3", GameOptions::default(), HoverCapture::ColonyShip);
        for (mode,label) in [(HoverCapture::Low,"a2-hover-low-16x9"),(HoverCapture::Other,"a2-hover-other-16x9")] {
            let vfs=sta_vfs::Vfs::mount_install(std::path::Path::new(&install),active_mod.as_deref()).unwrap();
            capture_one(vfs,profile,UVec2::new(1280,720),"gui_fed.cfg",label,GameOptions::default(),mode);
        }
    } else {
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref()).unwrap();
        capture_one(
            vfs, profile, UVec2::new(1280, 720), "gui_federation16x9.cfg",
            "fo4-hover-selected-16x9", GameOptions::default(), HoverCapture::SelectedHover,
        );
        if active_mod.as_deref() == Some("FO4") {
            for (mode, label) in [(HoverCapture::Avatar, "fo4-start-avatar"), (HoverCapture::WeaponIcons, "fo4-intrepid-system-icons")] {
                let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref()).unwrap();
                capture_one(vfs, profile, UVec2::new(1920, 1080), "gui_federation16x9.cfg", label, GameOptions::default(), mode);
            }
        }
    }
}

/// Armada I: the three separate resource panels (dilithium, crew, officers), from the
/// install's own `gui_*.cfg` and `gui_<race>.spr`, drawn in a windowless GPU frame.
/// Set `STA64_GPU_TESTS=1 STA64_TEST_A1_INSTALL=<Armada I root>`; `STA64_HUD_CAPTURE_DIR`
/// keeps the frames locally (they show game art: never commit them).
#[test]
fn installed_armada1_resource_readouts_render_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 STA64_TEST_A1_INSTALL=<Armada I root>");
        return;
    }
    let Some(install) = std::env::var_os("STA64_TEST_A1_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_A1_INSTALL=<Armada I root>");
        return;
    };
    for size in sizes(&[UVec2::new(1280, 720), UVec2::new(2560, 1080), UVec2::new(3840, 1080)]) {
    for (cfg, spr, label) in [
        ("gui_fed.cfg", "gui_federation.spr", "a1-fed"),
        ("gui_kli.cfg", "gui_klingon.spr", "a1-kli"),
        ("gui_rom.cfg", "gui_romulan.spr", "a1-rom"),
        ("gui_bor.cfg", "gui_borg.spr", "a1-bor"),
    ] {
        let label = &format!("{label}-{}x{}", size.x, size.y);
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), None).expect("installed A1 VFS");
        let layout = crate::game::gui_layout(&vfs, cfg).unwrap_or_else(|| panic!("installed {cfg} missing"));
        let stock = StockLayout::for_aspect(&layout, size.x as f32 / size.y as f32);
        assert!(stock.panel("resource").is_none(), "{label}: now has a resource panel");
        let hud_feed = feed(&vfs, GameOptions::default(), 0);
        let player = hud_feed.view.player(hud_feed.player).expect("capture player");
        let whole = |value: Fixed| (value.to_bits() / Fixed::ONE.to_bits()).to_string();
        // Expected values come from the content data, not from the view the HUD reads, so lost
        // starting values (the legacy RTS_CFG.h start, the officer cap) cannot pass as "" == "".
        // The feed is one Federation start for every cfg: only the layout differs per race.
        let db = hud_feed.content.as_ref().expect("content");
        let start = db.rules().legacy_start();
        let base_gain: i32 = hud_feed.view.units.iter().filter(|unit| unit.owner == hud_feed.player)
            .filter_map(|unit| db.unit(unit.def)?.a1_base_officers)
            .map(|base| base.gain).sum();
        let cap = db.a1_starting_officers().expect("officer cap") + base_gain;
        let available = whole((Fixed::from_int(cap) - player.officers[0]).max(Fixed::ZERO));
        let expected = [
            (2_usize, start.dilithium.expect("legacy start dilithium").to_string()),
            (0, start.crew.expect("legacy start crew").to_string()),
            (1, format!("{available}/{cap}")),
        ];
        assert!(expected.iter().all(|(_, v)| !v.is_empty()));
        eprintln!("{label}: expected readouts {expected:?}");
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
                .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::log::LogPlugin>(),
        );
        configure_sets(&mut app);
        let selection = hud_feed.selected.clone();
        app.insert_resource(ClearColor(Color::BLACK))
            .insert_resource(UiProfile::Armada2)
            .insert_resource(UiProfile::Armada2.theme())
            .insert_resource(Mounted(vfs))
            .insert_resource(GuiSkin { table: Some(spr.into()) })
            .insert_resource(HudConfig { gui: Some(layout), aspect: size.x as f32 / size.y as f32, fit: Default::default(), fit_scale: false, log: false })
            .insert_resource(UiScale(size.y as f32 / 1080.0))
            .insert_resource(hud_feed)
            .insert_resource(ExternalFeed)
            .insert_resource(Selection(selection))
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_plugins(HudPlugin)
            .add_systems(Update, aim_roots_hud.before(GameSet::Ui));
        let mut image = Image::new_target_texture(size.x, size.y, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
        image.asset_usage = RenderAssetUsages::RENDER_WORLD;
        let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        let camera = app
            .world_mut()
            .spawn((Camera2d, Camera { order: 1, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() }, RenderTarget::from(target.clone()), Msaa::Off))
            .id();
        app.insert_resource(CaptureCamera(camera));
        app.finish();
        app.cleanup();
        for _ in 0..16 {
            app.update();
        }
        let has_adapter = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned().or_else(|| {
            app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned())
        });
        let Some(adapter) = has_adapter else {
            eprintln!("SKIP {label}: no render adapter");
            return;
        };
        if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
            eprintln!("SKIP {label}: hardware adapter unavailable");
            return;
        }
        // Three readouts with the live values.
        for (id, want) in &expected {
            let actual = app
                .world_mut()
                .query::<(&super::info::StockText, &crate::ui::sprite_font::SpriteText)>()
                .iter(app.world())
                .find_map(|(kind, text)| matches!(kind, super::info::StockText::Resource(n) if n == id).then(|| text.text.clone()));
            assert_eq!(actual.as_deref(), Some(want.as_str()), "{label}: readout {id}");
        }
        // Each panel's background and icon sprite names resolve in the race's table.
        for prefix in ["officer", "crew", "dilithium"] {
            for key in [format!("{prefix}BackgroundPanel"), format!("{prefix}Icon")] {
                let wanted: Vec<String> = if key.ends_with("Icon") {
                    stock.name(&key).map(str::to_owned).into_iter().collect()
                } else {
                    stock.pieces(&key).into_iter().map(|p| p.name).collect()
                };
                assert!(!wanted.is_empty(), "{label}: {key} authored");
                for name in wanted {
                    let resolved = app
                        .world_mut()
                        .query::<(&GuiImage, &ImageNode)>()
                        .iter(app.world())
                        .any(|(image, _)| image.names.iter().any(|n| n.eq_ignore_ascii_case(&name)));
                    assert!(resolved, "{label}: sprite {name:?} ({key}) did not resolve in {spr}");
                    eprintln!("{label}: sprite {name} resolved");
                }
            }
        }
        let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
        let output = Arc::clone(&frames);
        app.world_mut().spawn(Screenshot::image(target)).observe(move |shot: On<ScreenshotCaptured>| {
            output.lock().unwrap().push(shot.image.clone());
        });
        for _ in 0..60 {
            app.update();
            if !frames.lock().unwrap().is_empty() {
                break;
            }
        }
        let shot = frames.lock().unwrap().first().cloned().unwrap_or_else(|| panic!("{label}: no GPU capture"));
        let rgba = shot.try_into_dynamic().expect("GPU image conversion").to_rgba8();
        // The numerals are drawn in the text colour inside each text rectangle (panel art
        // there is dark or not that colour), so lost text fails.
        let text_colour = UiProfile::Armada2.theme().text.to_srgba();
        let want = [text_colour.red, text_colour.green, text_colour.blue].map(|v| (v * 255.0).round() as i16);
        for prefix in ["officer", "crew", "dilithium"] {
            let area = stock.panel(prefix).expect("panel");
            let r = stock.at(Vec2::new(area.x, area.y), &format!("{prefix}TextArea")).expect("text area");
            let k = size.y as f32 / 1080.0;
            let (x0, x1) = ((r.x * k).max(0.0) as u32, ((r.x + r.w) * k).min(size.x as f32) as u32);
            let (y0, y1) = ((r.y * k).max(0.0) as u32, ((r.y + r.h) * k).min(size.y as f32) as u32);
            let matching = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| {
                let p = rgba.get_pixel(x, y);
                (0..3).all(|c| (p[c] as i16 - want[c]).abs() <= 48)
            }).count();
            eprintln!("{label}: {prefix} text rect {x0}..{x1} x {y0}..{y1}: {matching} text-colour pixels");
            assert!(matching >= 8, "{label}: {prefix} readout shows no text-colour pixels");
        }
        if let Ok(dir) = std::env::var("STA64_HUD_CAPTURE_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            rgba.save(std::path::Path::new(&dir).join(format!("stock-{label}.png"))).unwrap();
        }
    }
    }
}

#[allow(clippy::type_complexity)]
fn aim_roots_hud(
    mut commands: Commands,
    camera: Res<CaptureCamera>,
    roots: Query<Entity, (With<HudRoot>, Without<UiTargetCamera>)>,
) {
    for root in &roots {
        commands.entity(root).insert(UiTargetCamera(camera.0));
    }
}

/// Compact captions on scripted ships using installed definitions, never a window.
#[test]
fn installed_group_names_render_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") { return; }
    let root = std::env::var("STA64_TEST_INSTALL").expect("STA64_TEST_INSTALL");
    let mod_name = std::env::var("STA64_TEST_MOD").expect("STA64_TEST_MOD");
    let (profile, cfg, prefix) = match mod_name.as_str() {
        "STA2 Classic" | "STA2 Classic Mod" => (UiProfile::Armada2, "gui_fed.cfg", "classic-group"),
        "FO4" | "Fleet Ops 4.0" => (UiProfile::FleetOps, "gui_federation16x9.cfg", "fo4-group"),
        _ => panic!("use STA2 Classic or FO4 for the installed name regression"),
    };
    for (size, count, label) in [(UVec2::new(1920,1080), 2, format!("{prefix}-1080")),
        (UVec2::new(1280,720), 2, format!("{prefix}-720")),
        (UVec2::new(1920,1080), 16, format!("{prefix}-16-1080")),
        (UVec2::new(1920,1080), 17, format!("{prefix}-17-1080")),
        (UVec2::new(1920,1080), 30, format!("{prefix}-30-1080"))] {
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(&mod_name)).unwrap();
        capture_one(vfs, profile, size, cfg, &label, GameOptions::default(), HoverCapture::Group(count));
    }
}

/// UI-19: the Armada I stock HUD (`UiProfile::Armada1`) on the install's own content: the four
/// races at 4:3, 16:9 and 21:9, Centred and Stretched, in windowless GPU frames, plus a slot click
/// that must queue an order. The sim is the install's real starting state (a headless start).
/// For an A1Compat mount, set `STA64_TEST_A1_INSTALL` to the Roots install and
/// `STA64_TEST_A1_MOD` to its mod folder. `STA64_GPU_TESTS=1` opts in. Save
/// local frames with `STA64_HUD_CAPTURE_DIR`.
#[test]
fn installed_armada1_stock_hud_renders_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 STA64_TEST_A1_INSTALL=<Armada I root>");
        return;
    }
    let Some(install) = std::env::var_os("STA64_TEST_A1_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_A1_INSTALL=<Armada I root>");
        return;
    };
    let compat_mod = std::env::var("STA64_TEST_A1_MOD").ok();
    for fit in super::layout::HudFit::ALL {
    for size in sizes(&[UVec2::new(1024, 768), UVec2::new(1920, 1080), UVec2::new(2560, 1080)]) {
    for (cfg, spr, label) in [
        ("gui_fed.cfg", "gui_federation.spr", "a1-fed"),
        ("gui_kli.cfg", "gui_klingon.spr", "a1-kli"),
        ("gui_rom.cfg", "gui_romulan.spr", "a1-rom"),
        ("gui_bor.cfg", "gui_borg.spr", "a1-bor"),
    ] {
        if compat_mod.is_some() && cfg != "gui_fed.cfg" {
            continue;
        }
        let label = &format!("{label}-{}x{}-{}", size.x, size.y, fit.key());
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), compat_mod.as_deref()).expect("installed A1 VFS");
        if compat_mod.is_some() {
            assert!(sta_content::profile::a1_compat(&vfs), "the mod must require A1Compat");
            assert_eq!(
                crate::ui::profile::resolve_profile(Some(&vfs), Default::default(), &Default::default()),
                UiProfile::Armada1,
            );
        }
        let layout = crate::game::gui_layout(&vfs, cfg).unwrap_or_else(|| panic!("installed {cfg} missing"));
        let stock = StockLayout::for_aspect_fit(&layout, size.x as f32 / size.y as f32, fit);
        assert_eq!(stock.canvas, Vec2::new(640.0, 480.0), "{label}: one 640 x 480 sheet");
        let (mut hud_feed, sim) = feed_and_sim(&vfs, GameOptions::default(), 0);
        let mut upgrade_key = None;
        if compat_mod.is_some() {
            let db = hud_feed.content.as_ref().unwrap();
            let base = hud_feed.view.units.iter().find(|u| u.owner == hud_feed.player
                && db.unit(u.def).is_some_and(|d| d.a1_base_officers.is_some())).expect("starting starbase");
            hud_feed.selected = vec![base.id];
            upgrade_key = db.unit(base.def).unwrap().a1_base_officers.unwrap().upgrade;
            assert!(upgrade_key.is_some(), "the installed starbase has its race's officer upgrade");
        }
        let team = Color::srgb(0.25, 0.5, 1.0);
        let palette = crate::viewer::TeamColours::load(&vfs).with_local_colour(hud_feed.player, team, &[hud_feed.player]);
        let officer_content = hud_feed.content.clone();
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
                .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::log::LogPlugin>(),
        );
        configure_sets(&mut app);
        let selection = hud_feed.selected.clone();
        app.insert_resource(ClearColor(Color::BLACK))
            .insert_resource(UiProfile::Armada1)
            .insert_resource(UiProfile::Armada1.theme())
            .insert_resource(palette)
            .insert_resource(Mounted(vfs))
            .insert_resource(GuiSkin { table: Some(spr.into()) })
            .insert_resource(HudConfig { gui: Some(layout), aspect: size.x as f32 / size.y as f32, fit, fit_scale: false, log: false })
            .insert_resource(UiScale(size.y as f32 / 1080.0))
            .insert_resource(hud_feed)
            .insert_resource(ExternalFeed)
            .insert_resource(Selection(selection))
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_plugins(HudPlugin)
            .add_systems(Update, aim_roots_hud.before(GameSet::Ui));
        let mut image = Image::new_target_texture(size.x, size.y, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
        image.asset_usage = RenderAssetUsages::RENDER_WORLD;
        let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        let camera = app
            .world_mut()
            .spawn((Camera2d, Camera { order: 1, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() }, RenderTarget::from(target.clone()), Msaa::Off))
            .id();
        app.insert_resource(CaptureCamera(camera));
        app.finish();
        app.cleanup();
        for _ in 0..16 {
            app.update();
        }
        let adapter = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned().or_else(|| {
            app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned())
        });
        let Some(adapter) = adapter else {
            eprintln!("SKIP {label}: no render adapter");
            return;
        };
        if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
            eprintln!("SKIP {label}: hardware adapter unavailable");
            return;
        }
        // The slot map came from the install's header or its A1Compat defaults.
        let slots = app.world().resource::<super::StockState>().slots.clone().expect("slot map loaded");
        assert!(!slots.is_empty(), "{label}: game_cfg.h gave no slots");
        let cells = app.world_mut().query::<&super::super::panels::GridCell>().iter(app.world()).count();
        assert!(cells > 0, "{label}: no control buttons");
        if compat_mod.is_some() {
            use super::a1::info::LiveTint;
            let resolved_upgrade = app.world_mut()
                .query::<(&super::super::panels::GridCell, &ImageNode)>()
                .iter(app.world())
                .any(|(cell, _)| matches!(cell.0, super::super::Act::Build { what, site: false, .. } if Some(what) == upgrade_key));
            assert!(resolved_upgrade, "{label}: the officer upgrade sprite did not resolve in the main grid");
            let resolved_buttons = app.world_mut()
                .query::<(&super::super::panels::GridCell, &ImageNode)>()
                .iter(app.world())
                .count();
            assert!(resolved_buttons > 0, "{label}: command sprites did not resolve");
            let resolved_badge = app.world_mut()
                .query::<(&LiveTint, &ImageNode)>()
                .iter(app.world())
                .filter(|(tint, image)| matches!(tint, LiveTint::Race(_)) && image.color == team)
                .count();
            assert!(resolved_badge > 0, "{label}: team tint did not reach badge art");
            eprintln!("{label}: {resolved_buttons} resolved command sprites, {resolved_badge} tinted badge sprites");
        }
        // A click on a Stop slot (or the first ready order slot) queues an order.
        let slot_button = app
            .world_mut()
            .query::<(Entity, &super::super::panels::GridCell)>()
            .iter(app.world())
            .find(|(_, c)| matches!(c.0, super::super::Act::Stop | super::super::Act::Order(super::super::model::UnitOrder::Scout)))
            .map(|(e, _)| e);
        if let Some(e) = slot_button {
            crate::ui::theme::click(app.world_mut(), e);
            app.update();
            assert!(!app.world().resource::<crate::sim_link::CommandQueue>().0.is_empty(), "{label}: the slot click queued no order");
            app.world_mut().resource_mut::<crate::sim_link::CommandQueue>().0.clear();
        }
        let officer_check = upgrade_key.map(|what| {
            use crate::sim_link::{CommandQueue, ContentRes, SimLink};
            use sta_sim::command::{CommandKind, LocalRouter, Order};
            let db = officer_content.unwrap();
            let def = db.unit(what).unwrap();
            let ticks = def.build.ticks;
            let bonus = Fixed::from_int(def.a1_officer_upgrade_gain.unwrap());
            let sprite = format!("b_{}", def.odf);
            let button = app.world_mut()
                .query::<(Entity, &super::super::panels::GridCell, &ImageNode)>()
                .iter(app.world())
                .find_map(|(entity, cell, _)| match cell.0 {
                    super::super::Act::Build { builder, what: key, site: false, .. } if key == what => Some((entity, builder)),
                    _ => None,
                }).expect("rendered officer upgrade");
            crate::ui::theme::click(app.world_mut(), button.0);
            app.update();
            let orders = std::mem::take(&mut app.world_mut().resource_mut::<CommandQueue>().0);
            assert!(matches!(orders.as_slice(), [CommandKind::Order(Order::Build { builder, what: key, at: None })] if *builder == button.1 && *key == what), "{label}: {orders:?}");
            let player = PlayerId(0);
            let mut link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), player);
            let starting_cap = link.curr().player(player).unwrap().officers[1];
            for order in orders { link.submit(order); }
            let events = link.tick().unwrap();
            assert!(!events.iter().any(|e| matches!(e, sta_sim::SimEvent::Rejected { .. })), "{label}: {events:?}");
            for _ in 0..ticks / 4 { link.tick().unwrap(); }
            assert_eq!(link.curr().player(player).unwrap().officers[1], starting_cap);
            app.world_mut().remove_resource::<ExternalFeed>();
            app.insert_resource(ContentRes(db)).insert_resource(link);
            for _ in 0..4 { app.update(); }
            let resolved_job = app.world_mut().query::<(&GuiImage, &ImageNode)>().iter(app.world())
                .any(|(art, _)| art.names.contains(&sprite));
            assert!(resolved_job, "{label}: officer job sprite did not resolve on the speed rail");
            use super::super::status_gpu::LiveValue;
            let progress = app.world_mut().query::<(&LiveValue, &Node)>().iter(app.world())
                .any(|(kind, node)| matches!(kind, LiveValue::QueueProgress(0, _)) && matches!(node.width, Val::Px(w) if w > 1.0));
            assert!(progress, "{label}: no live officer build progress");
            (ticks, starting_cap + bonus)
        });
        let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
        let output = Arc::clone(&frames);
        app.world_mut().spawn(Screenshot::image(target)).observe(move |shot: On<ScreenshotCaptured>| {
            output.lock().unwrap().push(shot.image.clone());
        });
        for _ in 0..60 {
            app.update();
            if !frames.lock().unwrap().is_empty() {
                break;
            }
        }
        let shot = frames.lock().unwrap().first().cloned().unwrap_or_else(|| panic!("{label}: no GPU capture"));
        let rgba = shot.try_into_dynamic().expect("GPU image conversion").to_rgba8();
        let coloured = rgba.pixels().filter(|p| p[0] > 20 || p[1] > 20 || p[2] > 20).count();
        eprintln!("{label}: {cells} control buttons, {coloured} nonblack pixels");
        assert!(coloured > 500, "{label}: HUD did not visibly render");
        if let Ok(dir) = std::env::var("STA64_HUD_CAPTURE_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            rgba.save(std::path::Path::new(&dir).join(format!("stock-{label}.png"))).unwrap();
        }
        if let Some((ticks, expected_cap)) = officer_check {
            let mut link = app.world_mut().resource_mut::<crate::sim_link::SimLink>();
            for _ in 0..ticks { link.tick().unwrap(); }
            assert_eq!(link.curr().player(PlayerId(0)).unwrap().officers[1], expected_cap);
        }
    }
    }
    }
}

/// The Armada I speed rail's TRANSPORT button on the install's own content, in a windowless GPU frame: a
/// crewed own ship selected, the button drawn at the rail's place from the sheet, the press arming the pick.
/// Set `STA64_GPU_TESTS=1 STA64_TEST_A1_INSTALL=<Armada I root>`; `STA64_HUD_CAPTURE_DIR` keeps the frame
/// locally (it shows game art: never commit it).
#[test]
fn installed_armada1_speed_rail_transport_button_renders_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 STA64_TEST_A1_INSTALL=<Armada I root>");
        return;
    }
    let Some(install) = std::env::var_os("STA64_TEST_A1_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_A1_INSTALL=<Armada I root>");
        return;
    };
    let (cfg, spr) = ("gui_fed.cfg", "gui_federation.spr");
    let size = UVec2::new(1920, 1080);
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), None).expect("installed A1 VFS");
    let layout = crate::game::gui_layout(&vfs, cfg).expect("installed gui cfg");
    let fit = super::layout::HudFit::ALL[0];
    let stock = StockLayout::for_aspect_fit(&layout, size.x as f32 / size.y as f32, fit);
    let mut hud_feed = feed(&vfs, GameOptions::default(), 1);
    // The selection: an own ship with crew in the view (the shared start has a builder ship).
    let crewed: Vec<_> = hud_feed.view.units.iter().filter(|u| u.owner == PlayerId(0) && u.crew.is_some()).map(|u| (u.id, u.crew)).collect();
    eprintln!("own crewed units: {crewed:?}");
    let pick = crewed.first().expect("an own unit with crew").0;
    hud_feed.selected = vec![pick];
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    configure_sets(&mut app);
    app.insert_resource(ClearColor(Color::BLACK))
        .insert_resource(UiProfile::Armada1)
        .insert_resource(UiProfile::Armada1.theme())
        .insert_resource(Mounted(vfs))
        .insert_resource(GuiSkin { table: Some(spr.into()) })
        .insert_resource(HudConfig { gui: Some(layout), aspect: size.x as f32 / size.y as f32, fit, fit_scale: false, log: false })
        .insert_resource(UiScale(size.y as f32 / 1080.0))
        .insert_resource(Selection(hud_feed.selected.clone()))
        .insert_resource(hud_feed)
        .insert_resource(ExternalFeed)
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(HudPlugin)
        .add_systems(Update, aim_roots_hud.before(GameSet::Ui));
    let mut image = Image::new_target_texture(size.x, size.y, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let camera = app
        .world_mut()
        .spawn((Camera2d, Camera { order: 1, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() }, RenderTarget::from(target.clone()), Msaa::Off))
        .id();
    app.insert_resource(CaptureCamera(camera));
    app.finish();
    app.cleanup();
    for _ in 0..16 {
        app.update();
    }
    let adapter = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned().or_else(|| {
        app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned())
    });
    let Some(adapter) = adapter else {
        eprintln!("SKIP: no render adapter");
        return;
    };
    if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
        eprintln!("SKIP: hardware adapter unavailable");
        return;
    }
    // The button: the install's own art name, at the rail's place computed from the sheet.
    let want = super::a1::speed::rect(&stock, 6).expect("the sheet has the rail keys");
    let found: Vec<_> = app
        .world_mut()
        .query::<(&super::super::panels::GridCell, &GuiButton, &Node)>()
        .iter(app.world())
        .filter(|(c, _, _)| c.0 == super::super::Act::Target(super::super::model::TargetMode::Transport))
        .map(|(_, b, n)| (b.names.clone(), n.left, n.top, n.width))
        .collect();
    assert_eq!(found, vec![(vec!["b_transport".to_owned()], Val::Px(want.x), Val::Px(want.y), Val::Px(want.w))], "one transport button on the rail");
    let button = app.world_mut().query::<(Entity, &super::super::panels::GridCell)>().iter(app.world()).find(|(_, c)| c.0 == super::super::Act::Target(super::super::model::TargetMode::Transport)).map(|(e, _)| e).unwrap();
    crate::ui::theme::click(app.world_mut(), button);
    app.update();
    assert_eq!(app.world().resource::<super::super::HudState>().targeting, Some(super::super::model::TargetMode::Transport), "the press arms the pick");
    let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    let output = Arc::clone(&frames);
    app.world_mut().spawn(Screenshot::image(target)).observe(move |shot: On<ScreenshotCaptured>| {
        output.lock().unwrap().push(shot.image.clone());
    });
    for _ in 0..60 {
        app.update();
        if !frames.lock().unwrap().is_empty() {
            break;
        }
    }
    let shot = frames.lock().unwrap().first().cloned().expect("a GPU capture");
    let rgba = shot.try_into_dynamic().expect("GPU image conversion").to_rgba8();
    let coloured = rgba.pixels().filter(|p| p[0] > 20 || p[1] > 20 || p[2] > 20).count();
    eprintln!("rail frame: {coloured} nonblack pixels");
    assert!(coloured > 500, "the HUD rendered");
    if let Ok(dir) = std::env::var("STA64_HUD_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        rgba.save(std::path::Path::new(&dir).join("rail-transport-a1-fed-1920x1080.png")).unwrap();
    }
}

/// UI-42: hovering an Armada I build button shows its `needs:` line, in a windowless GPU frame.
/// The unit is the install's real starting construction ship; the tooltip is STA64's own (the
/// original's place for it is not established). Set `STA64_GPU_TESTS=1
/// STA64_TEST_A1_INSTALL=<Armada I root>`; `STA64_HUD_CAPTURE_DIR` keeps the frame locally (it
/// shows game art: never commit it).
#[test]
fn installed_armada1_build_costs_tooltip_renders_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 STA64_TEST_A1_INSTALL=<Armada I root>");
        return;
    }
    let Some(install) = std::env::var_os("STA64_TEST_A1_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_A1_INSTALL=<Armada I root>");
        return;
    };
    let size = UVec2::new(1920, 1080);
    let label = "a1-build-costs-tooltip-1920x1080";
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), None).expect("installed A1 VFS");
    let layout = crate::game::gui_layout(&vfs, "gui_fed.cfg").expect("installed gui_fed.cfg");
    let strings = crate::ui::i18n::Strings::load(&vfs, "english");
    let words = strings.clone();
    let hud_feed = feed(&vfs, GameOptions::default(), 0);
    let db = hud_feed.content.clone().expect("installed content");
    // Every own unit with a build list: the starting construction ship, and a station or research
    // building when the start has one.
    let builders: Vec<sta_sim::EntityId> = hud_feed
        .view
        .units
        .iter()
        .filter(|u| u.owner == hud_feed.player && db.unit(u.def).is_some_and(|d| !d.build.list.is_empty()))
        .map(|u| u.id)
        .collect();
    assert!(!builders.is_empty(), "the start has a builder");
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    configure_sets(&mut app);
    let selection = hud_feed.selected.clone();
    app.insert_resource(ClearColor(Color::BLACK))
        .insert_resource(UiProfile::Armada1)
        .insert_resource(UiProfile::Armada1.theme())
        .insert_resource(Mounted(vfs))
        .insert_resource(strings)
        .insert_resource(GuiSkin { table: Some("gui_federation.spr".into()) })
        .insert_resource(HudConfig { gui: Some(layout), aspect: size.x as f32 / size.y as f32, fit_scale: false, ..default() })
        .insert_resource(UiScale(size.y as f32 / 1080.0))
        .insert_resource(hud_feed)
        .insert_resource(ExternalFeed)
        .insert_resource(Selection(selection))
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(HudPlugin)
        .add_systems(Update, aim_roots_hud.before(GameSet::Ui));
    let mut image = Image::new_target_texture(size.x, size.y, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let camera = app
        .world_mut()
        .spawn((Camera2d, Camera { order: 1, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() }, RenderTarget::from(target.clone()), Msaa::Off))
        .id();
    app.insert_resource(CaptureCamera(camera));
    app.finish();
    app.cleanup();
    for _ in 0..8 {
        app.update();
    }
    let adapter = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned().or_else(|| {
        app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned())
    });
    let Some(adapter) = adapter else {
        eprintln!("SKIP {label}: no render adapter");
        return;
    };
    if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
        eprintln!("SKIP {label}: hardware adapter unavailable");
        return;
    }
    // The rule, for every build button of every builder, all computed from the data at run time: the
    // tooltip holds the string table's own "needs" word, and for each cost the unit's definition
    // charges, its amount and its resource word (the table's for dilithium, the resource model's
    // otherwise); the officers amount when it takes officers.
    let require = words.tooltip_text("GUI_CP_REQUIRE").to_owned();
    assert_ne!(require, "GUI_CP_REQUIRE", "{label}: the install's table has the word");
    let mut first = None;
    let mut with_line = 0;
    for builder in &builders {
        app.world_mut().resource_mut::<HudFeed>().selected = vec![*builder];
        app.world_mut().resource_mut::<Selection>().0 = vec![*builder];
        app.world_mut().resource_mut::<super::super::HudState>().page = super::super::Page::Build;
        for _ in 0..4 {
            app.update();
        }
        let resources = super::super::model::resource_model(app.world().resource::<HudFeed>());
        let cells: Vec<(Entity, sta_sim::ContentKey)> = app
            .world_mut()
            .query::<(Entity, &super::super::panels::GridCell)>()
            .iter(app.world())
            .filter_map(|(e, c)| match c.0 {
                super::super::Act::Build { what, .. } => Some((e, what)),
                _ => None,
            })
            .collect();
        for (cell, what) in cells {
            let short = app.world().get::<crate::ui::notices::Tooltip>(cell).expect("tooltip").short.clone();
            let needs = super::a1::costs::needs(&db, what).expect("the item's definition");
            if needs.slots.is_empty() && needs.officers == 0 {
                continue;
            }
            with_line += 1;
            assert!(short.contains(&require), "{label}: {short:?} lacks {require:?}");
            for (slot, amount) in &needs.slots {
                assert!(short.contains(&amount.to_string()), "{label}: {short:?} lacks the amount {amount}");
                let word = if *slot == sta_content::resources::DILITHIUM {
                    words.tooltip_text("GUI_CP_DILITHIUM_RES").to_lowercase()
                } else {
                    resources.name(*slot).to_lowercase()
                };
                assert!(short.to_lowercase().contains(&word), "{label}: {short:?} lacks the word {word:?}");
            }
            if needs.officers > 0 {
                assert!(short.contains(&needs.officers.to_string()), "{label}: {short:?} lacks the officers amount");
            }
            first.get_or_insert((*builder, what, short));
        }
    }
    eprintln!("{label}: {} builder(s), {with_line} priced build button(s) checked", builders.len());
    // Back to the first priced button's builder to hover it.
    let (builder, what, short) = first.expect("at least one priced build button");
    app.world_mut().resource_mut::<HudFeed>().selected = vec![builder];
    app.world_mut().resource_mut::<Selection>().0 = vec![builder];
    for _ in 0..4 {
        app.update();
    }
    let item = app
        .world_mut()
        .query::<(Entity, &super::super::panels::GridCell)>()
        .iter(app.world())
        .find(|(_, c)| matches!(&c.0, super::super::Act::Build { what: w, .. } if *w == what))
        .map(|(e, _)| e)
        .expect("the button again");
    assert_eq!(app.world().get::<crate::ui::notices::Tooltip>(item).unwrap().short, short);
    let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    let output = Arc::clone(&frames);
    app.world_mut().spawn(Screenshot::image(target)).observe(move |shot: On<ScreenshotCaptured>| {
        output.lock().unwrap().push(shot.image.clone());
    });
    for _ in 0..60 {
        *app.world_mut().resource_mut::<crate::ui::notices::TipHover>() =
            crate::ui::notices::TipHover { target: Some(item), rest: crate::ui::notices::TIP_SHORT_SECS + 0.1 };
        app.update();
        if !frames.lock().unwrap().is_empty() {
            break;
        }
    }
    let shown = app
        .world_mut()
        .query::<&crate::ui::sprite_font::SpriteText>()
        .iter(app.world())
        .any(|t| t.text == short);
    assert!(shown, "{label}: the tooltip text was not displayed");
    let shot = frames.lock().unwrap().first().cloned().unwrap_or_else(|| panic!("{label}: no GPU capture"));
    let rgba = shot.try_into_dynamic().expect("GPU image conversion").to_rgba8();
    if let Ok(dir) = std::env::var("STA64_HUD_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        rgba.save(std::path::Path::new(&dir).join(format!("stock-{label}.png"))).unwrap();
    }
}

/// Local font-sharpness check: the stock HUD of the installed game at the frame sizes in
/// `STA64_HUD_SIZES` (default: a 3440 x 1440 and a 2752 x 1152 window), kept in
/// `STA64_HUD_CAPTURE_DIR` for measuring glyph edges. Set `STA64_GPU_TESTS=1 STA64_HUD_TESTS=1
/// STA64_TEST_INSTALL STA64_HUD_PROFILE=a2|fo` and optional `STA64_TEST_MOD`. Frames show game
/// art: never commit them.
#[test]
fn installed_stock_hud_font_edges_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") || std::env::var("STA64_HUD_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 STA64_HUD_TESTS=1 STA64_TEST_INSTALL STA64_HUD_PROFILE=a2|fo");
        return;
    }
    let install = std::env::var("STA64_TEST_INSTALL").expect("STA64_TEST_INSTALL");
    let active_mod = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let (profile, cfg, tag) = match std::env::var("STA64_HUD_PROFILE").as_deref() {
        Ok("a2") => (UiProfile::Armada2, "gui_fed.cfg", "a2"),
        Ok("fo") => (UiProfile::FleetOps, "gui_federation16x9.cfg", "fo4"),
        _ => panic!("set STA64_HUD_PROFILE=a2 or fo"),
    };
    for size in sizes(&[UVec2::new(3440, 1440), UVec2::new(2752, 1152)]) {
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref()).expect("installed stock VFS");
        capture_one(vfs, profile, size, cfg, &format!("{tag}-edges-{}x{}", size.x, size.y), GameOptions::default(), HoverCapture::Selected);
    }
}

/// Local font-sharpness check: mission-start titles as `ui::presentation` draws them (a typed
/// title, a subtitle, a centred title; blue and white) over black at the sizes in
/// `STA64_HUD_SIZES`, in the installed game's sprite fonts. Set `STA64_GPU_TESTS=1
/// STA64_HUD_TESTS=1 STA64_TEST_INSTALL STA64_HUD_PROFILE=a1|a2|fo`, optional `STA64_TEST_MOD`
/// and `STA64_HUD_CAPTURE_DIR`. Frames show game art: never commit them.
#[test]
fn installed_titles_render_on_gpu() {
    use crate::ui::sprite_font::{SpriteFontPlugin, SpriteTextStyle, sprite_text, sprite_text_styled};
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") || std::env::var("STA64_HUD_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 STA64_HUD_TESTS=1 STA64_TEST_INSTALL STA64_HUD_PROFILE=a1|a2|fo");
        return;
    }
    let install = std::env::var("STA64_TEST_INSTALL").expect("STA64_TEST_INSTALL");
    let active_mod = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let (profile, tag) = match std::env::var("STA64_HUD_PROFILE").as_deref() {
        Ok("a1") => (UiProfile::Armada1, "a1"),
        Ok("a2") => (UiProfile::Armada2, "a2"),
        Ok("fo") => (UiProfile::FleetOps, "fo4"),
        _ => panic!("set STA64_HUD_PROFILE=a1, a2 or fo"),
    };
    for size in sizes(&[UVec2::new(3440, 1440), UVec2::new(2752, 1152)]) {
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref()).expect("installed VFS");
        let theme = profile.theme();
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
                .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
                .disable::<bevy::gilrs::GilrsPlugin>()
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::log::LogPlugin>(),
        );
        configure_sets(&mut app);
        app.insert_resource(ClearColor(Color::BLACK))
            .insert_resource(profile)
            .insert_resource(theme.clone())
            .insert_resource(Mounted(vfs))
            .insert_resource(UiScale(size.y as f32 / 1080.0))
            .add_plugins(SpriteFontPlugin);
        let mut image = Image::new_target_texture(size.x, size.y, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
        image.asset_usage = RenderAssetUsages::RENDER_WORLD;
        let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        let camera = app
            .world_mut()
            .spawn((Camera2d, Camera { clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() }, RenderTarget::from(target.clone()), Msaa::Off))
            .id();
        let role = |s: f32| crate::ui::theme::font_role(&theme, s);
        let blue = crate::ui::presentation::title_colour(false);
        let white = crate::ui::presentation::title_colour(true);
        let blue = Color::srgb(blue[0], blue[1], blue[2]);
        let white = Color::srgb(white[0], white[1], white[2]);
        let (head, body) = (theme.heading_size, theme.text_size);
        let at = |left: f32, top: f32| Node { position_type: PositionType::Absolute, left: Val::Percent(left), top: Val::Percent(top), max_width: Val::Percent(80.0), ..default() };
        let root = app
            .world_mut()
            .spawn((Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, UiTargetCamera(camera)))
            .id();
        let style = |s: f32| SpriteTextStyle::role(role(s)).wrap_to_parent();
        app.world_mut().spawn((at(3.1, 4.2), ChildOf(root), children![sprite_text_styled("Sample Title 0123456789", head, blue, style(head))]));
        app.world_mut().spawn((at(3.1, 12.0), ChildOf(root), children![sprite_text_styled("Sample subtitle line 0123456789", body, white, style(body))]));
        app.world_mut().spawn((
            Node { position_type: PositionType::Absolute, left: Val::Percent(0.0), width: Val::Percent(100.0), justify_content: JustifyContent::Center, top: Val::Percent(50.0), ..default() },
            ChildOf(root),
            children![sprite_text_styled("Centred Title", head, blue, style(head))],
        ));
        // Plain `sprite_text` (the Refit-layout HUD's: crisp, size-preserving choice) and a
        // `scaled` caption (stock group tiles: the only path still at a fractional scale).
        app.world_mut().spawn((at(3.1, 20.0), ChildOf(root), children![sprite_text("Plain sprite text 0123456789", 21.0, white)]));
        app.world_mut().spawn((
            at(3.1, 28.0),
            ChildOf(root),
            children![sprite_text_styled("Scaled caption 0123456789", 17.0, white, SpriteTextStyle { scaled: true, ..SpriteTextStyle::role(role(17.0)) })],
        ));
        app.finish();
        app.cleanup();
        for _ in 0..16 {
            app.update();
        }
        let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
        let output = Arc::clone(&frames);
        app.world_mut().spawn(Screenshot::image(target)).observe(move |shot: On<ScreenshotCaptured>| {
            output.lock().unwrap().push(shot.image.clone());
        });
        for _ in 0..60 {
            app.update();
            if !frames.lock().unwrap().is_empty() {
                break;
            }
        }
        let shot = frames.lock().unwrap().first().cloned().expect("title frame");
        let rgba = shot.try_into_dynamic().expect("title frame image").to_rgba8();
        let ink = rgba.pixels().filter(|p| p[0] > 20 || p[1] > 20 || p[2] > 20).count();
        eprintln!("{tag} titles {}x{}: {ink} lit pixels", size.x, size.y);
        assert!(ink > 200, "{tag}: no title ink");
        if let Ok(dir) = std::env::var("STA64_HUD_CAPTURE_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            rgba.save(std::path::Path::new(&dir).join(format!("titles-{tag}-{}x{}.png", size.x, size.y))).unwrap();
        }
    }
}

/// Local font-size check of the Refit (non-stock) HUD on an install's fonts: draws it for one
/// selected unit and for a group of constructors at the sizes in `STA64_HUD_SIZES`, prints one
/// `label:` line per sprite text (styled or plain, text, requested size, drawn glyph height in physical px, own
/// and parent box widths, `OVERFLOW` where the text is wider or taller than a parent box) and
/// keeps the frame in `STA64_HUD_CAPTURE_DIR`. Set `STA64_GPU_TESTS=1 STA64_HUD_TESTS=1
/// STA64_TEST_INSTALL` and optional `STA64_TEST_MOD`. Frames show game art: never commit them.
#[test]
fn installed_refit_hud_labels_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") || std::env::var("STA64_HUD_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 STA64_HUD_TESTS=1 STA64_TEST_INSTALL");
        return;
    }
    let install = std::env::var("STA64_TEST_INSTALL").expect("STA64_TEST_INSTALL");
    let active_mod = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    for size in sizes(&[UVec2::new(3440, 1440)]) {
        for group in [0usize, 4] {
            let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref()).expect("installed VFS");
            let mut hud_feed = feed(&vfs, GameOptions::default(), group);
            if group > 0 {
                hud_feed.selected = hud_feed.view.units.iter().rev().filter(|u| u.owner == hud_feed.player
                    && hud_feed.content.as_ref().unwrap().unit(u.def).is_some_and(|d| d.class.0 == "constructionrig"))
                    .take(group).map(|u| u.id).collect();
            }
            let selection = hud_feed.selected.clone();
            let radar_feed = RadarFeed {
                view: hud_feed.view.clone(),
                player: hud_feed.player,
                selected: selection.clone(),
                content: hud_feed.content.clone(),
                camera: None,
            };
            let profile = UiProfile::Lcars;
            let mut app = App::new();
            app.add_plugins(
                DefaultPlugins
                    .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
                    .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
                    .disable::<bevy::gilrs::GilrsPlugin>()
                    .disable::<bevy::winit::WinitPlugin>()
                    .disable::<bevy::log::LogPlugin>(),
            );
            configure_sets(&mut app);
            app.insert_resource(ClearColor(Color::BLACK))
                .insert_resource(profile)
                .insert_resource(crate::input::hover::HoveredUnit(None))
                .insert_resource(profile.theme())
                .insert_resource(Mounted(vfs))
                .insert_resource(HudConfig { gui: None, aspect: size.x as f32 / size.y as f32, fit: env_fit(), fit_scale: false, log: false })
                .insert_resource(UiScale(size.y as f32 / 1080.0))
                .insert_resource(hud_feed)
                .insert_resource(radar_feed)
                .insert_resource(ExternalFeed)
                .insert_resource(Selection(selection))
                .init_resource::<ButtonInput<MouseButton>>()
                .init_resource::<ButtonInput<KeyCode>>()
                .add_plugins(HudPlugin)
                .add_systems(Update, aim_roots.before(GameSet::Ui));
            let mut image = Image::new_target_texture(size.x, size.y, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
            image.asset_usage = RenderAssetUsages::RENDER_WORLD;
            let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
            let camera = app
                .world_mut()
                .spawn((Camera2d, Camera { order: 1, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() }, RenderTarget::from(target.clone()), Msaa::Off))
                .id();
            app.insert_resource(CaptureCamera(camera));
            app.finish();
            app.cleanup();
            for _ in 0..24 {
                app.update();
            }
            let ui = size.y as f32 / 1080.0;
            let mut rows = Vec::new();
            let mut q = app.world_mut().query::<(Entity, &crate::ui::sprite_font::SpriteText, &ComputedNode, Option<&ChildOf>, Option<&Children>, Has<crate::ui::sprite_font::SpriteTextStyle>)>();
            for (_, text, node, parent, kids, styled) in q.iter(app.world()) {
                let drawn = kids
                    .and_then(|k| k.iter().find_map(|c| app.world().get::<ComputedNode>(c)))
                    .map_or(0.0, |c| c.size().y);
                let (pw, ph) = parent
                    .and_then(|p| app.world().get::<ComputedNode>(p.parent()))
                    .map_or((0.0, 0.0), |c| (c.size().x, c.size().y));
                let over = pw > 0.0 && ph > 0.0 && (node.size().x > pw + 1.0 || node.size().y > ph + 1.0);
                rows.push(format!(
                    "label: group={group} {} {:?} want={:.1}px drawn_h={:.0}px own={:.0}x{:.0} parent={:.0}x{:.0}{}",
                    if styled { "styled" } else { "plain" }, text.text, text.size * ui, drawn, node.size().x, node.size().y, pw, ph, if over { " OVERFLOW" } else { "" }
                ));
            }
            rows.sort();
            for r in &rows {
                eprintln!("{r}");
            }
            let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
            let output = Arc::clone(&frames);
            app.world_mut().spawn(Screenshot::image(target)).observe(move |shot: On<ScreenshotCaptured>| {
                output.lock().unwrap().push(shot.image.clone());
            });
            for _ in 0..60 {
                app.update();
                if !frames.lock().unwrap().is_empty() {
                    break;
                }
            }
            let shot = frames.lock().unwrap().first().cloned().expect("refit frame");
            let rgba = shot.try_into_dynamic().expect("refit frame image").to_rgba8();
            if let Ok(dir) = std::env::var("STA64_HUD_CAPTURE_DIR") {
                std::fs::create_dir_all(&dir).unwrap();
                rgba.save(std::path::Path::new(&dir).join(format!("refit-g{group}-{}x{}.png", size.x, size.y))).unwrap();
            }
        }
    }
}
