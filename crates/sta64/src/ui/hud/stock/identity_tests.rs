//! Selected identity rows through the actual stock HUD, with authored CFG only.

use super::*;

fn identity_content(builder: bool, odf: &str) -> Arc<ContentDb> {
    let base = content_with_build_count(usize::from(builder));
    let mut out = ContentDb::builder()
        .profile(base.profile())
        .features(base.features().clone());
    for weapon in base.weapons() {
        out = out.weapon(weapon.clone());
    }
    for unit in base.units() {
        let mut unit = unit.clone();
        if unit.odf == "fx_cruiser" {
            unit.raw =
                Arc::new(sta_odf::resolve("fx_cruiser", &mut |_| Some(odf.to_owned())).unwrap());
        }
        out = out.unit(unit);
    }
    Arc::new(out.build().unwrap())
}

const ODF: &str = "possibleCraftNames = \"USS Étoile\" \"USS Aura\"\npossibleCaptainNames = \"Capt. Zoë\" \"Capt. Rey\"\npossibleCraftRegistry = \"NCC-É 1701\" \"NX-42\"\n";

fn row(app: &mut App, field: info::IdentityField) -> Option<(String, Color, Node)> {
    app.world_mut()
        .query::<(&info::IdentityText, &SpriteText, &ChildOf)>()
        .iter(app.world())
        .find_map(|(kind, text, parent)| {
            (kind.0 == field).then(|| {
                (
                    text.text.clone(),
                    text.color,
                    app.world().get::<Node>(parent.parent()).unwrap().clone(),
                )
            })
        })
}

fn rows(app: &mut App) -> usize {
    app.world_mut()
        .query::<&info::IdentityText>()
        .iter(app.world())
        .count()
}

#[test]
fn selected_identity_is_cosmetic_to_content_digest() {
    assert_eq!(
        identity_content(false, ODF).hash(),
        identity_content(false, "").hash()
    );
}

fn assert_rect(app: &App, node: &Node, x: f32, y: f32, w: f32, h: f32) {
    let layout = app
        .world()
        .resource::<StockState>()
        .layout
        .as_ref()
        .unwrap();
    let origin = layout.info_origin(layout::info_variant(&model::info(
        app.world().resource::<HudFeed>(),
    )));
    let scale = layout.scale;
    let round = |n: f32| (n + 0.5).floor();
    assert_eq!(
        (node.left, node.top, node.width, node.height),
        (
            Val::Px(origin.x + round(x * scale)),
            Val::Px(origin.y + round(y * scale)),
            Val::Px(round(w * scale)),
            Val::Px(round(h * scale))
        )
    );
}

#[test]
fn medium_and_tall_rows_keep_selected_name_row_and_their_own_cfg() {
    let cfg = inherited_config(
        "infoBuildCaptainTextArea = 20 120 310 25\ninfoBuildCaptainTextColor = 0 0 1 1\n",
        "infoSingleCaptainTextArea = 10 20 300 20\ninfoSingleRegistryTextArea = 10 50 300 20\ninfoSingleCaptainTextColor = 1 0 0 1\ninfoSingleRegistryTextColor = 0 1 0 1\nshipNameColor = 1 0 1 1\n",
    );
    for builder in [false, true] {
        let db = identity_content(builder, ODF);
        let mut app = app_with_content_and_config(db, cfg.clone(), UiProfile::Armada2);
        let Info::Single(single) = model::info(app.world().resource::<HudFeed>()) else {
            panic!("single");
        };
        let captain = row(&mut app, info::IdentityField::Captain).unwrap();
        let registry = row(&mut app, info::IdentityField::Registry).unwrap();
        match single.name.as_str() {
            "USS Étoile" => {
                assert_eq!(captain.0, "Capt. Zoë");
                assert_eq!(registry.0, "NCC-É 1701");
            }
            "USS Aura" => {
                assert_eq!(captain.0, "Capt. Rey");
                assert_eq!(registry.0, "NX-42");
            }
            other => panic!("wrong selected name: {other}"),
        }
        assert_eq!(rows(&mut app), 2);
        if builder {
            assert_rect(&app, &captain.2, 20.0, 120.0, 310.0, 25.0);
            assert_eq!(captain.1, Color::srgba(0.0, 0.0, 1.0, 1.0));
        } else {
            assert_rect(&app, &captain.2, 10.0, 20.0, 300.0, 20.0);
            assert_eq!(captain.1, Color::srgba(1.0, 0.0, 0.0, 1.0));
        }
        // Tall registry inherits the medium rectangle and colour independently.
        assert_rect(&app, &registry.2, 10.0, 50.0, 300.0, 20.0);
        assert_eq!(registry.1, Color::srgba(0.0, 1.0, 0.0, 1.0));
        for aspect in [4.0 / 3.0, 21.0 / 9.0] {
            app.world_mut().resource_mut::<HudConfig>().aspect = aspect;
            app.update();
            let captain = row(&mut app, info::IdentityField::Captain).unwrap();
            let registry = row(&mut app, info::IdentityField::Registry).unwrap();
            let (x, y, w, h) = if builder {
                (20.0, 120.0, 310.0, 25.0)
            } else {
                (10.0, 20.0, 300.0, 20.0)
            };
            assert_rect(&app, &captain.2, x, y, w, h);
            assert_rect(&app, &registry.2, 10.0, 50.0, 300.0, 20.0);
        }
    }
}

#[test]
fn legacy_areas_and_colour_fallbacks_are_independent() {
    let cfg = inherited_config(
        "infoBuildRegistryTextColor = 0 0 1 1\n",
        "captainName = 11 21 230 19\nshipRegistry = 12 44 220 18\ncaptainNameColor = 1 0 0 1\nshipRegistryColor = 0 1 0 1\ninfoTextColor = 1 1 0 1\nshipNameColor = 1 0 1 1\n",
    );
    for builder in [false, true] {
        let mut app = app_with_content_and_config(
            identity_content(builder, ODF),
            cfg.clone(),
            UiProfile::Armada2,
        );
        let captain = row(&mut app, info::IdentityField::Captain).unwrap();
        let registry = row(&mut app, info::IdentityField::Registry).unwrap();
        assert_rect(&app, &captain.2, 11.0, 21.0, 230.0, 19.0);
        assert_rect(&app, &registry.2, 12.0, 44.0, 220.0, 18.0);
        assert_eq!(captain.1, Color::srgba(1.0, 0.0, 0.0, 1.0));
        assert_eq!(
            registry.1,
            if builder {
                Color::srgba(0.0, 0.0, 1.0, 1.0)
            } else {
                Color::srgba(0.0, 1.0, 0.0, 1.0)
            }
        );
    }
}

#[test]
fn explicit_disabled_areas_do_not_revive_legacy_or_medium_rows() {
    let cfg = inherited_config(
        "infoBuildCaptainTextArea = 9 9 0 20\ninfoBuildRegistryTextArea = 9 9 -1 20\n",
        "infoSingleCaptainTextArea = 9 9 0 20\ninfoSingleRegistryTextArea = 9 9 -1 20\ncaptainName = 1 1 200 20\nshipRegistry = 1 25 200 20\n",
    );
    for builder in [false, true] {
        let mut app = app_with_content_and_config(
            identity_content(builder, ODF),
            cfg.clone(),
            UiProfile::Armada2,
        );
        assert_eq!(rows(&mut app), 0);
    }
    let cfg = inherited_config(
        "infoBuildCaptainTextArea = 9 9 0 20\n",
        "infoSingleCaptainTextArea = 4 8 120 20\ninfoSingleRegistryTextArea = 4 30 120 20\n",
    );
    let mut app = app_with_content_and_config(identity_content(true, ODF), cfg, UiProfile::Armada2);
    assert!(row(&mut app, info::IdentityField::Captain).is_none());
    assert!(row(&mut app, info::IdentityField::Registry).is_some());
}

#[test]
fn blank_companions_and_group_or_no_selection_emit_no_rows() {
    let cfg = inherited_config(
        "",
        "infoSingleCaptainTextArea = 10 20 300 20\ninfoSingleRegistryTextArea = 10 50 300 20\n",
    );
    let mut app = app_with_content_and_config(
        identity_content(
            false,
            "possibleCraftNames = \"Only\"\npossibleCaptainNames = \"\"\npossibleCraftRegistry = \"\"\n",
        ),
        cfg.clone(),
        UiProfile::Armada2,
    );
    assert_eq!(rows(&mut app), 0);
    let mut absent = app_with_content_and_config(
        identity_content(false, "possibleCraftNames = \"Only\"\n"),
        cfg.clone(),
        UiProfile::Armada2,
    );
    assert_eq!(rows(&mut absent), 0);
    let mut app =
        app_with_content_and_config(identity_content(false, ODF), cfg, UiProfile::Armada2);
    assert_eq!(rows(&mut app), 2);
    {
        let mut feed = app.world_mut().resource_mut::<HudFeed>();
        feed.selected.clear();
    }
    app.update();
    assert_eq!(rows(&mut app), 0);
    // A group, even when the same unit appears twice, uses group tiles only.
    app.world_mut().resource_mut::<HudFeed>().selected = vec![EntityId(1), EntityId(1)];
    app.update();
    assert_eq!(rows(&mut app), 0);
}
