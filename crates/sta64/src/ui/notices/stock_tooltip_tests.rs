//! The Armada II stock tooltip on authored data: placement, box, delays, hover clock, looks.
use super::*;
use crate::app::configure_sets;
use crate::ui::hud::HudConfig;
use crate::ui::hud::stock::layout::HudFit;
use crate::ui::hud::stock::tooltip_text::WorldTipParts;
use crate::ui::profile::UiProfile;
use crate::ui::sprite_font::{LoadedFont, SpriteFont, SpriteFonts};
use crate::ui::stock_font::{StockCanvas, StockTextCanvas};
use crate::ui::stock_tooltip::*;
use bevy::window::PrimaryWindow;

/// An authored font: ASCII glyphs `w` texels wide, `h` texels tall.
fn authored_font(w: u32, h: u32) -> SpriteFont {
    let n = 95;
    let mut t = String::from("sprite_table\n@referenceWidth=256\n@referenceHeight=256\n\n@animation f_uv\n");
    t += &format!("offset {n} 0.0 step\n@keyframes\n");
    for i in 0..n {
        t += &format!("{}\t0\t#{}\n", (i * w) % 250, (0x20 + i as u8) as char);
    }
    t += &format!("\n@animation f_w\nwidth {n} 0.0 step\n@keyframes\n");
    for _ in 0..n {
        t += &format!("{w}\n");
    }
    t += &format!("\npagea\ttex\t\t0\t0\t0\t{h}\t@anim=f_uv\t@anim=f_w\n");
    SpriteFont::parse("f", &t).expect("authored font")
}

fn fonts() -> SpriteFonts {
    let mut f = SpriteFonts { stock_family: true, ..default() };
    f.fonts.insert("fontfinal4_10".into(), LoadedFont::new(authored_font(10, 14), vec![(Handle::default(), UVec2::new(256, 256))]));
    f
}

fn canvas(hud: Vec2, window: Vec2, fit: HudFit) -> StockTextCanvas {
    StockTextCanvas(Some(StockCanvas { hud, window, fit }))
}

const BG: &str = "screenWidth = 800\nscreenHeight = 600\ntooltipBackgroundColor = 0.25 0.5 0.75\ntooltipTextColor = 1 1 0\n";

fn rig(profile: UiProfile, canvas: StockTextCanvas, window: Vec2, cursor: Vec2) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    configure_sets(&mut app);
    app.insert_resource(profile).insert_resource(fonts()).insert_resource(canvas).add_plugins(NoticePlugin);
    let (cfg, _) = sta_content::GuiCfg::parse("t.cfg", BG);
    app.insert_resource(HudConfig { gui: Some(cfg.layout_auto()), ..default() });
    let mut w = Window::default();
    w.resolution.set(window.x, window.y);
    w.set_cursor_position(Some(cursor));
    app.world_mut().spawn((w, PrimaryWindow));
    app.update();
    app
}

fn show(app: &mut App, text: &str) {
    let e = app.world_mut().spawn(Tooltip::new(text)).id();
    *app.world_mut().resource_mut::<TipHover>() = TipHover { target: Some(e), rest: 10.0 };
    app.update();
}

fn node(app: &mut App) -> Node {
    app.world_mut().query_filtered::<&Node, With<TipBox>>().single(app.world()).unwrap().clone()
}

fn px(v: Val) -> f32 {
    let Val::Px(p) = v else { panic!("{v:?}") };
    p
}

#[test]
fn placement_goldens_with_authored_numbers() {
    let s = Vec2::splat(0.5);
    let text = Vec2::new(200.0, 60.0);
    // Below the pointer by the offset, text on the pixel grid.
    let p = tooltip_place(Vec2::new(300.0, 400.0), text, s);
    assert_eq!(p.text, Vec2::new(300.0, 456.0));
    assert_eq!(p.rect, Rect { min: Vec2::new(294.0, 454.0), max: Vec2::new(506.0, 522.0) });
    // Snap: an odd unit rounds to the next whole pixel (halves up).
    assert_eq!(tooltip_place(Vec2::new(301.0, 401.0), text, s).text, Vec2::new(302.0, 458.0));
    // Left clamp.
    assert_eq!(tooltip_place(Vec2::new(4.0, 300.0), text, s).text.x, 12.0);
    // Right clamp to the viewport: the box ends short of the right edge.
    let r = tooltip_place(Vec2::new(1500.0, 300.0), Vec2::new(300.0, 60.0), s);
    assert_eq!((r.text.x, r.rect.max.x), (1288.0, 1594.0));
    // Flip above near the bottom; the box bottom sits just below the pointer.
    let f = tooltip_place(Vec2::new(300.0, 1100.0), text, s);
    assert_eq!(f.text.y, 1036.0);
    assert_eq!(f.rect.max.y, 1102.0);
    // The flip starts where the text and its margin reach the bottom.
    assert_eq!(tooltip_place(Vec2::new(300.0, 1075.0), text, s).text.y, 1132.0);
    assert_eq!(tooltip_place(Vec2::new(300.0, 1076.0), text, s).text.y, 1012.0);
    // Top clamp.
    assert_eq!(tooltip_place(Vec2::new(300.0, -100.0), text, s).text.y, 0.0);
    assert_eq!(tooltip_place(Vec2::new(300.0, 1190.0), Vec2::new(200.0, 1300.0), s).text.y, 0.0);
}

#[test]
fn the_box_is_the_text_block_with_its_padding_on_whole_pixels() {
    let s = Vec2::splat(0.5);
    let place = tooltip_place(Vec2::new(500.0, 200.0), Vec2::new(400.0, 60.0), s);
    let at = physical_box(&place, Vec2::new(200.0, 28.0), s, Vec2::ZERO);
    assert_eq!(at.pen, Vec2::new(251.0, 129.0));
    assert_eq!(at.rect.size(), Vec2::new(206.0, 32.0));
    assert_eq!((at.rect.min.x, at.rect.min.y), (247.0, 128.0));
}

#[test]
fn text_measures_and_wraps_at_the_width_with_authored_advances() {
    let f = fonts();
    let c = canvas(Vec2::new(800.0, 600.0), Vec2::new(800.0, 600.0), HudFit::Centred);
    assert_eq!(measure(&f, &c, 1.0, "Hello"), Some(Vec2::new(50.0, 14.0)));
    // 550 units are 275 px: 27 glyphs of 10 px.
    let long = "aaaaaaaaaa bbbbbbbbbb cccccccccc dddddddddd";
    let m = measure(&f, &c, 1.0, long).unwrap();
    assert_eq!(m.y, 28.0);
    assert!(m.x <= 275.0 && m.x >= 200.0, "{m:?}");
    // Explicit breaks stay.
    assert_eq!(measure(&f, &c, 1.0, "a\nb\nc").unwrap().y, 42.0);
    // A character the font lacks leaves the plain path.
    assert_eq!(measure(&f, &c, 1.0, "caf\u{4e2d}"), None);
    // No stock rule without a HUD or outside the family.
    assert_eq!(measure(&f, &StockTextCanvas::default(), 1.0, "a"), None);
}

#[test]
fn the_stock_box_has_no_border_the_cfg_colours_and_sits_below_the_pointer() {
    let size = Vec2::new(800.0, 600.0);
    let mut app = rig(UiProfile::Armada2, canvas(size, size, HudFit::Centred), size, Vec2::new(250.0, 180.0));
    show(&mut app, "Hello");
    let n = node(&mut app);
    assert_eq!(n.display, Display::Flex);
    assert_eq!((px(n.left), px(n.top)), (247.0, 208.0));
    assert_eq!((px(n.width), px(n.height)), (56.0, 18.0));
    assert_eq!(n.border, UiRect::ZERO);
    assert_eq!((px(n.padding.left), px(n.padding.top)), (4.0, 1.0));
    let bg = app.world_mut().query_filtered::<&BackgroundColor, With<TipBox>>().single(app.world()).unwrap().0;
    assert_eq!(bg, Color::srgba(0.25, 0.5, 0.75, 1.0));
    let words = app.world_mut().query_filtered::<&SpriteText, With<TipText>>().single(app.world()).unwrap().color;
    assert_eq!(words, Color::srgba(1.0, 1.0, 0.0, 1.0));
    assert!(app.world().resource::<TipStock>().0);
    // It follows the pointer.
    app.world_mut().query::<&mut Window>().single_mut(app.world_mut()).unwrap().set_cursor_position(Some(Vec2::new(300.0, 100.0)));
    app.update();
    let n = node(&mut app);
    assert_eq!((px(n.left), px(n.top)), (297.0, 128.0));
}

#[test]
fn a_centred_wide_hud_keeps_the_box_inside_the_hud_area_and_a_stretched_one_inside_the_window() {
    // 4:3 HUD of 1440x1080 centred in 2520x1080: the HUD spans x 540..1980.
    let (hud, window) = (Vec2::new(1440.0, 1080.0), Vec2::new(2520.0, 1080.0));
    let mut app = rig(UiProfile::Armada2, canvas(hud, window, HudFit::Centred), window, Vec2::new(2500.0, 300.0));
    show(&mut app, "Hello");
    let n = node(&mut app);
    let right = px(n.left) + px(n.width);
    assert!(right <= 1980.0 && right >= 1980.0 - 6.0 * 0.9 - 1.0, "right edge {right}");
    // Left of the HUD area: the left clamp, 12 units in from the HUD's own edge.
    app.world_mut().query::<&mut Window>().single_mut(app.world_mut()).unwrap().set_cursor_position(Some(Vec2::new(10.0, 300.0)));
    app.update();
    let n = node(&mut app);
    assert!(px(n.left) >= 540.0 + (12.0 - 6.0) * 0.9 - 1.0, "left edge {}", px(n.left));
    // Stretched: the HUD is the window, one unit is 0.8 px across and 0.6 px down.
    let w = Vec2::new(1280.0, 720.0);
    let mut app = rig(UiProfile::Armada2, canvas(w, w, HudFit::Stretched), w, Vec2::new(1270.0, 100.0));
    show(&mut app, "Hello");
    let n = node(&mut app);
    let right = px(n.left) + px(n.width);
    assert!(right <= 1280.0 && right >= 1280.0 - 6.0 * 0.8 - 1.0, "right edge {right}");
    // Near the bottom it flips above the pointer.
    app.world_mut().query::<&mut Window>().single_mut(app.world_mut()).unwrap().set_cursor_position(Some(Vec2::new(600.0, 700.0)));
    app.update();
    let n = node(&mut app);
    assert!(px(n.top) + px(n.height) < 700.0 + 5.0, "{:?}", n.top);
}

#[test]
fn other_looks_and_profiles_keep_the_plain_box() {
    let size = Vec2::new(800.0, 600.0);
    for profile in [UiProfile::Lcars, UiProfile::Armada1, UiProfile::FleetOps] {
        let mut app = rig(profile, canvas(size, size, HudFit::Centred), size, Vec2::new(250.0, 180.0));
        show(&mut app, "Hello");
        let n = node(&mut app);
        assert!(!app.world().resource::<TipStock>().0, "{profile:?}");
        assert_eq!(n.border, UiRect::left(Val::Px(4.0)), "{profile:?}");
        assert_eq!(n.width, Val::Auto);
        // The plain rule: 16 and 20 UI units off the pointer.
        assert_eq!((px(n.left), px(n.top)), (266.0, 200.0), "{profile:?}");
    }
    // The Armada II look without a HUD canvas keeps it too.
    let mut app = rig(UiProfile::Armada2, StockTextCanvas::default(), size, Vec2::new(250.0, 180.0));
    show(&mut app, "Hello");
    assert!(!app.world().resource::<TipStock>().0);
    assert_eq!(node(&mut app).border, UiRect::left(Val::Px(4.0)));
}

/// The Armada II look without the stock font rule (Fleet Operations 3.x, Dominion War, another install)
/// keeps today's behaviour: the plain box, STA64's delays and the name-only unit tooltip at once.
#[test]
fn the_armada_ii_look_without_the_stock_font_rule_changes_nothing() {
    let size = Vec2::new(800.0, 600.0);
    let mut app = rig(UiProfile::Armada2, canvas(size, size, HudFit::Centred), size, Vec2::new(250.0, 180.0));
    app.world_mut().resource_mut::<SpriteFonts>().stock_family = false;
    app.update();
    assert_eq!(*app.world().resource::<TipDelays>(), TipDelays::default());
    // A unit under the pointer: the plain name at once, not the composed text.
    app.world_mut().insert_resource(UnitTip { target: Some(sta_sim::EntityId(1)), plain: "Plain".into(), parts: parts(), rest: 0.0 });
    app.update();
    let words = app.world_mut().query_filtered::<&SpriteText, With<TipText>>().single(app.world()).unwrap().text.clone();
    assert_eq!(words, "Plain");
    let n = node(&mut app);
    assert_eq!(n.display, Display::Flex);
    assert!(!app.world().resource::<TipStock>().0);
    assert_eq!(n.border, UiRect::left(Val::Px(4.0)));
    assert_eq!((px(n.left), px(n.top)), (266.0, 200.0));
    // A button's tooltip takes the old delay, too.
    *app.world_mut().resource_mut::<UnitTip>() = UnitTip::default();
    let e = app.world_mut().spawn(Tooltip::new("Btn")).id();
    *app.world_mut().resource_mut::<TipHover>() = TipHover { target: Some(e), rest: 0.4 };
    app.update();
    assert_eq!(node(&mut app).display, Display::Flex);
}

#[test]
fn no_stock_tooltip_during_a_cutscene() {
    let size = Vec2::new(800.0, 600.0);
    let mut app = rig(UiProfile::Armada2, canvas(size, size, HudFit::Centred), size, Vec2::new(250.0, 180.0));
    app.init_resource::<crate::ui::presentation::CineractiveState>();
    show(&mut app, "Hello");
    assert_eq!(node(&mut app).display, Display::Flex);
    app.world_mut().resource_mut::<crate::ui::presentation::CineractiveState>().on = true;
    app.update();
    assert_eq!(node(&mut app).display, Display::None);
}

#[test]
fn the_delays_come_from_the_component_definition_with_the_defaults_for_absent_keys() {
    let odf = |t: &str| sta_odf::resolve("component", &mut |_| Some(t.to_owned())).unwrap();
    assert_eq!(delays_from(None), TipDelays::STOCK);
    assert_eq!(delays_from(Some(&odf("other = 1\n"))), TipDelays::STOCK);
    assert_eq!(delays_from(Some(&odf("tooltipDelay = 0.25\n"))), TipDelays { short: 0.25, verbose: TipDelays::STOCK.verbose });
    assert_eq!(delays_from(Some(&odf("tooltipDelay = 0.1\nverboseTooltipDelay = 1.5\n"))), TipDelays { short: 0.1, verbose: 1.5 });
    assert_eq!(delays_from(Some(&odf("tooltipDelay = -2\n"))).short, TipDelays::STOCK.short);
    assert!(TipDelays::STOCK.short > 0.0 && TipDelays::STOCK.short < TipDelays::STOCK.verbose);
}

#[test]
fn the_delays_resource_follows_the_look() {
    let size = Vec2::new(800.0, 600.0);
    let app = rig(UiProfile::Armada2, canvas(size, size, HudFit::Centred), size, Vec2::ZERO);
    assert_eq!(*app.world().resource::<TipDelays>(), TipDelays::STOCK);
    let app = rig(UiProfile::FleetOps, canvas(size, size, HudFit::Centred), size, Vec2::ZERO);
    assert_eq!(*app.world().resource::<TipDelays>(), TipDelays::default());
}

#[test]
fn the_two_steps_count_from_the_start_of_the_hover_and_the_verbose_replaces_the_short() {
    let d = TipDelays { short: 0.5, verbose: 3.0 };
    let t = Tooltip::new("S").verbose("V");
    let mut seen = Vec::new();
    for rest in [0.0, 0.49, 0.5, 2.99, 3.0, 9.0] {
        seen.push(t.text_at_with(rest, &d));
    }
    assert_eq!(seen, [None, None, Some("S"), Some("S"), Some("V"), Some("V")]);
    // No verbose text: the short one stays.
    assert_eq!(Tooltip::new("S").text_at_with(9.0, &d), Some("S"));
}

fn parts() -> WorldTipParts {
    WorldTipParts { craft: true, name: Some("N".into()), class_name: "C".into(), tooltip: Some("T".into()), verbose: Some("VT".into()) }
}

#[test]
fn the_world_clock_resets_on_a_new_target_and_not_on_movement_within_it() {
    let d = TipDelays { short: 0.5, verbose: 3.0 };
    let look = |k: &str| k.to_owned();
    let id = |n| sta_sim::EntityId(n);
    let mut tip = UnitTip::default();
    tip.step(Some((id(1), "N".into(), parts())), 0.1);
    assert_eq!(tip.rest, 0.0, "the first frame of a hover starts at zero");
    assert_eq!(tip.text(true, &d, &look), None);
    tip.step(Some((id(1), "N".into(), parts())), 0.5);
    assert_eq!(tip.text(true, &d, &look).as_deref(), Some("N - C"));
    tip.step(Some((id(1), "N".into(), parts())), 2.6);
    assert_eq!(tip.text(true, &d, &look), Some("N - T:\nVT".to_owned()));
    // Another unit restarts both steps.
    tip.step(Some((id(2), "M".into(), parts())), 0.1);
    assert_eq!(tip.text(true, &d, &look), None);
    // Leaving the unit clears the tooltip.
    tip.step(None, 0.1);
    assert_eq!(tip, UnitTip::default());
    assert_eq!(tip.text(true, &d, &look), None);
}

#[test]
fn a_world_target_without_verbose_text_keeps_its_short_text_and_other_looks_show_the_name_at_once() {
    let d = TipDelays { short: 0.5, verbose: 3.0 };
    let look = |k: &str| k.to_owned();
    let bare = WorldTipParts { tooltip: None, verbose: None, ..parts() };
    let mut tip = UnitTip { target: Some(sta_sim::EntityId(3)), plain: "Plain".into(), parts: bare, rest: 9.0 };
    assert_eq!(tip.text(true, &d, &look).as_deref(), Some("N - C"));
    tip.rest = 0.0;
    assert_eq!(tip.text(false, &d, &look).as_deref(), Some("Plain"));
}

fn words(app: &mut App) -> String {
    app.world_mut().query_filtered::<&SpriteText, With<TipText>>().single(app.world()).unwrap().text.clone()
}

fn style(app: &mut App) -> SpriteTextStyle {
    app.world_mut().query_filtered::<&SpriteTextStyle, With<TipText>>().single(app.world()).unwrap().clone()
}

fn hover(app: &mut App, tip: Tooltip, rest: f32) {
    let e = app.world_mut().spawn(tip).id();
    *app.world_mut().resource_mut::<TipHover>() = TipHover { target: Some(e), rest };
    app.update();
}

fn stock_rig() -> App {
    let size = Vec2::new(800.0, 600.0);
    rig(UiProfile::Armada2, canvas(size, size, HudFit::Centred), size, Vec2::new(250.0, 180.0))
}

#[test]
fn the_underline_rules_sit_on_the_marked_glyph_cells_across_wraps_and_alignment() {
    let f = fonts();
    let font = &f.fonts["fontfinal4_10"].font;
    let at = |text: &str, marks: &[usize], wrap: Option<f32>, align| font.underline_rects(text, marks, Vec2::ONE, true, wrap, align);
    // Glyphs 10 wide and 14 high: one pixel on the bottom row of each marked cell.
    assert_eq!(at("Hi there", &[0, 3], None, crate::ui::sprite_font::Align::Left), [[0.0, 13.0, 10.0, 1.0], [30.0, 13.0, 10.0, 1.0]]);
    // A wrap drops the space it breaks at: the mark on it gives no rule, the next line's marks count from its start.
    assert_eq!(at("aaa bbb", &[3, 4, 6], Some(35.0), crate::ui::sprite_font::Align::Left), [[0.0, 27.0, 10.0, 1.0], [20.0, 27.0, 10.0, 1.0]]);
    // An explicit break likewise.
    assert_eq!(at("a\nb", &[2], None, crate::ui::sprite_font::Align::Left), [[0.0, 27.0, 10.0, 1.0]]);
    // Centred text is aligned inside the wrap width first.
    assert_eq!(at("ab", &[0], Some(100.0), crate::ui::sprite_font::Align::Center), [[40.0, 13.0, 10.0, 1.0]]);
    // No marks, or marks past the text: none; thicker at a larger scale.
    assert!(at("ab", &[], None, crate::ui::sprite_font::Align::Left).is_empty() && at("ab", &[9], None, crate::ui::sprite_font::Align::Left).is_empty());
    assert_eq!(font.underline_rects("a", &[0], Vec2::splat(2.0), true, None, crate::ui::sprite_font::Align::Left), [[0.0, 26.0, 20.0, 2.0]]);
}

#[test]
fn under_the_stock_look_a_button_text_drops_the_marker_and_the_marked_glyphs_are_handed_to_the_text() {
    let mut app = stock_rig();
    hover(&mut app, Tooltip::new("^Zap <^Z>"), 10.0);
    assert_eq!(words(&mut app), "Zap <Z>");
    assert_eq!(app.world().resource::<TipMarks>().0, [0, 5]);
    assert_eq!(style(&mut app).underline, [0, 5]);
    // A text without a marker is untouched and marks nothing.
    hover(&mut app, Tooltip::new("Plain <F2>"), 10.0);
    assert_eq!(words(&mut app), "Plain <F2>");
    assert!(app.world().resource::<TipMarks>().0.is_empty());
    assert!(style(&mut app).underline.is_empty());
}

#[test]
fn a_bound_key_fills_the_slot_and_the_string_keeps_its_letter_where_there_is_no_binding() {
    use crate::input::keys::{Action, KeyBindings};
    let mut app = stock_rig();
    app.insert_resource(KeyBindings::default());
    // Stop's default is S: the string's letter is replaced by the binding.
    hover(&mut app, Tooltip::new("^Halt <^H>").bound_to(Action::Stop), 10.0);
    assert_eq!(words(&mut app), "Halt <S>");
    assert_eq!(app.world().resource::<TipMarks>().0, [0, 6]);
    app.world_mut().resource_mut::<KeyBindings>().rebind(Action::Stop, KeyCode::Digit4);
    app.update();
    assert_eq!(words(&mut app), "Halt <4>");
    // A key that is no letter or digit leaves the string's own letter.
    app.world_mut().resource_mut::<KeyBindings>().rebind(Action::Stop, KeyCode::F5);
    app.update();
    assert_eq!(words(&mut app), "Halt <H>");
    // No action named: the string's letter.
    hover(&mut app, Tooltip::new("^Halt <^H>"), 10.0);
    assert_eq!(words(&mut app), "Halt <H>");
}

#[test]
fn the_group_buttons_take_the_string_table_keys_when_it_has_them_and_their_own_labels_when_not() {
    use crate::ui::i18n::Strings;
    let mut app = stock_rig();
    app.insert_resource(Strings::from_pairs("english", &[("K_SHORT", "Short key text"), ("K_LONG", "Long key text: more.")]));
    let tip = || Tooltip::new("Own label").verbose("Own long label").stock_keys("K_SHORT", "K_LONG");
    hover(&mut app, tip(), 1.0);
    assert_eq!(words(&mut app), "Short key text");
    hover(&mut app, tip(), 10.0);
    assert_eq!(words(&mut app), "Long key text: more.");
    // The table has the short key but not the verbose one: the short text stays.
    app.insert_resource(Strings::from_pairs("english", &[("K_SHORT", "Short key text")]));
    app.update();
    assert_eq!(words(&mut app), "Short key text");
    // The table lacks the keys: today's labels.
    app.insert_resource(Strings::from_pairs("english", &[]));
    app.update();
    assert_eq!(words(&mut app), "Own long label");
    hover(&mut app, tip(), 1.0);
    assert_eq!(words(&mut app), "Own label");
}

#[test]
fn a_build_item_is_composed_from_the_table_under_the_stock_look() {
    use crate::ui::hud::stock::tooltip_text::{BuildTipParts, CostPart};
    use crate::ui::i18n::Strings;
    let mut app = stock_rig();
    app.insert_resource(Strings::from_pairs(
        "english",
        &[("K_TIP", "Zed Class"), ("K_VERB", "About zeds."), ("K_LABEL", "<F9>"), ("GUI_CP_DILITHIUM_RES", "dust"), ("GUI_CP_CREW_RES", "hands")],
    ));
    let parts = BuildTipParts {
        tooltip: "K_TIP".into(),
        verbose: Some("K_VERB".into()),
        hotkey: Some("K_LABEL".into()),
        cost: vec![
            CostPart { key: "GUI_CP_DILITHIUM_RES", fallback: "dilithium".into(), amount: 30.0 },
            CostPart { key: "GUI_CP_METAL_RES", fallback: "metal".into(), amount: 0.0 },
            CostPart { key: "GUI_CP_CREW_RES", fallback: "crew".into(), amount: 5.0 },
        ],
    };
    let tip = || Tooltip::new("K_TIP").stock_build(parts.clone());
    hover(&mut app, tip(), 1.0);
    assert_eq!(words(&mut app), "Zed Class: <F9>\n( 30 d / 5 h )");
    hover(&mut app, tip(), 10.0);
    assert_eq!(words(&mut app), "Zed Class: <F9>\nAbout zeds.\n( 30 dust / 5 hands )");
    // Before the short delay nothing shows, as for every tooltip.
    hover(&mut app, tip(), 0.1);
    assert_eq!(node(&mut app).display, Display::None);
}

/// Every other look keeps the text as the button gave it: no marker handling, no hotkey slot, no
/// string-table keys, no build composition.
#[test]
fn other_looks_keep_the_buttons_text_markers_and_keys_as_they_are() {
    use crate::input::keys::{Action, KeyBindings};
    use crate::ui::hud::stock::tooltip_text::BuildTipParts;
    use crate::ui::i18n::Strings;
    let size = Vec2::new(800.0, 600.0);
    let mut cases: Vec<App> = [UiProfile::Lcars, UiProfile::Armada1, UiProfile::FleetOps]
        .into_iter()
        .map(|p| rig(p, canvas(size, size, HudFit::Centred), size, Vec2::new(250.0, 180.0)))
        .collect();
    let mut no_rule = stock_rig();
    no_rule.world_mut().resource_mut::<SpriteFonts>().stock_family = false;
    cases.push(no_rule);
    for app in &mut cases {
        app.insert_resource(KeyBindings::default());
        app.insert_resource(Strings::from_pairs("english", &[("K_SHORT", "Key text"), ("K_TIP", "Zed Class")]));
        hover(app, Tooltip::new("^Halt <^H>").bound_to(Action::Stop), 10.0);
        assert_eq!(words(app), "^Halt <^H>");
        assert!(app.world().resource::<TipMarks>().0.is_empty());
        hover(app, Tooltip::new("Own label").stock_keys("K_SHORT", "K_SHORT"), 10.0);
        assert_eq!(words(app), "Own label");
        let parts = BuildTipParts { tooltip: "K_TIP".into(), ..default() };
        hover(app, Tooltip::new("K_TIP").stock_build(parts), 10.0);
        assert_eq!(words(app), "Zed Class");
        assert!(!app.world().resource::<TipStock>().0 && style(app).underline.is_empty());
    }
}

fn scratch_install(name: &str, delay: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sta64-tip-{}-{name}", std::process::id()));
    let file = dir.join("Data/odf/other/component.odf");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, format!("tooltipDelay = {delay}\n")).unwrap();
    dir
}

/// A rebuild of the stock HUD or a live resize changes the canvas, not the content: the component
/// definition is read again only when the mounted content changes.
#[test]
fn the_delays_are_read_once_per_mount_not_on_every_hud_rebuild() {
    let (a, b) = (scratch_install("a", "0.25"), scratch_install("b", "0.75"));
    let mut app = stock_rig();
    assert_eq!(app.world().resource::<TipDelayCache>().reads, 0, "no mount, no read");
    app.insert_resource(crate::viewer::Mounted(sta_vfs::Vfs::mount_install(&a, None).unwrap()));
    app.update();
    assert_eq!(app.world().resource::<TipDelayCache>().reads, 1);
    assert_eq!(app.world().resource::<TipDelays>().short, 0.25);
    // The HUD rebuilds (a new canvas value each time), the window resizes: no further read.
    for w in [1280.0, 1920.0, 800.0] {
        let hud = Vec2::new(w, w * 0.75);
        app.insert_resource(canvas(hud, hud, HudFit::Centred));
        app.update();
    }
    assert_eq!(app.world().resource::<TipDelayCache>().reads, 1, "a rebuild without a content change re-reads nothing");
    assert_eq!(app.world().resource::<TipDelays>().short, 0.25);
    // The look leaving the stock gate and returning does not read again either.
    app.insert_resource(UiProfile::Lcars);
    app.update();
    assert_eq!(*app.world().resource::<TipDelays>(), TipDelays::default());
    app.insert_resource(UiProfile::Armada2);
    app.update();
    assert_eq!(app.world().resource::<TipDelayCache>().reads, 1);
    assert_eq!(app.world().resource::<TipDelays>().short, 0.25);
    // New content: read again.
    app.insert_resource(crate::viewer::Mounted(sta_vfs::Vfs::mount_install(&b, None).unwrap()));
    app.update();
    assert_eq!(app.world().resource::<TipDelayCache>().reads, 2);
    assert_eq!(app.world().resource::<TipDelays>().short, 0.75);
    for dir in [a, b] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Measured at one window size: a click on the hovered button, or a key press, neither dismisses its
/// tooltip nor restarts its clock. The stock look draws the tooltip from the hover alone: no system
/// of it reads the buttons or the keys.
#[test]
fn a_click_or_a_key_press_neither_dismisses_the_tooltip_nor_restarts_its_clock() {
    use bevy::input::ButtonInput;
    let mut app = stock_rig();
    app.init_resource::<ButtonInput<KeyCode>>().init_resource::<ButtonInput<MouseButton>>();
    let tip = || Tooltip::new("Short").verbose("Long");
    hover(&mut app, tip(), 1.0);
    let target = app.world().resource::<TipHover>().target;
    assert_eq!(words(&mut app), "Short");
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyZ);
    app.update();
    {
        let h = app.world().resource::<TipHover>();
        assert_eq!(h.target, target, "the hover target is kept");
        assert!(h.rest >= 1.0, "the clock is not restarted: {}", h.rest);
    }
    assert_eq!(node(&mut app).display, Display::Flex);
    assert_eq!(words(&mut app), "Short");
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>().release(MouseButton::Left);
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::KeyZ);
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>().clear();
    app.update();
    assert_eq!(node(&mut app).display, Display::Flex);
    // The verbose step still arrives on the unchanged clock.
    app.world_mut().resource_mut::<TipHover>().rest = 3.5;
    app.update();
    assert_eq!(words(&mut app), "Long");
}

/// Installed assets are local-only: the mounted component definition gives the two delays and the
/// string table resolves a class's world tooltip keys and the two separators. Compares structure,
/// commits no text. Set `STA64_TOOLTIP_TESTS=1 STA64_TEST_INSTALL` (and optionally `STA64_TEST_MOD`).
#[test]
fn installed_delays_and_world_tooltip_strings_resolve() {
    if std::env::var("STA64_TOOLTIP_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_TOOLTIP_TESTS=1 STA64_TEST_INSTALL and optional STA64_TEST_MOD");
        return;
    }
    let install = std::env::var("STA64_TEST_INSTALL").expect("STA64_TEST_INSTALL");
    let active_mod = std::env::var("STA64_TEST_MOD").ok().filter(|s| !s.is_empty());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref()).unwrap();
    let component = read_component(&vfs).expect("the mounted component definition");
    let d = delays_from(Some(&component));
    eprintln!("mounted delays: short {} verbose {}", d.short, d.verbose);
    assert!(d.short > 0.0 && d.verbose > 0.0 && d.short < d.verbose, "{d:?}");

    let strings = crate::ui::i18n::Strings::load(&vfs, "english");
    let look = |k: &str| strings.tooltip_text(k).to_owned();
    for key in [crate::ui::hud::stock::tooltip_text::CLASS_SEPARATE_KEY, crate::ui::hud::stock::tooltip_text::TOOLTIP_SEPARATE_KEY] {
        assert_ne!(look(key), key, "the table has the separator {key}");
    }
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let mut found = None;
    for (stem, _) in index.iter() {
        let Ok(odf) = sta_odf::resolve_layered(stem, &mut |n, v| index.load(&vfs, n, v)) else { continue };
        let key = |k: &str| odf.str(k).map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned);
        if let (Some(class_name), Some(tooltip), Some(verbose)) = (key("unitName"), key("tooltip"), key("verboseTooltip"))
            && [&tooltip, &verbose].iter().all(|k| look(k) != **k)
        {
            found = Some(WorldTipParts { craft: true, name: Some("Authored Name".into()), class_name, tooltip: Some(tooltip), verbose: Some(verbose) });
            break;
        }
    }
    let p = found.expect("an installed class whose unitName, tooltip and verboseTooltip keys all resolve");
    use crate::ui::hud::stock::tooltip_text::*;
    let sep = look(CLASS_SEPARATE_KEY);
    assert_eq!(world_short(&p, &look), format!("Authored Name {sep} {}", look(&p.class_name)));
    let verbose = world_verbose(&p, &look);
    let want = format!("Authored Name {sep} {}{}\n{}", look(p.tooltip.as_deref().unwrap()), look(TOOLTIP_SEPARATE_KEY), look(p.verbose.as_deref().unwrap()));
    assert_eq!(verbose, want);
    assert!(verbose.lines().count() >= 2);
}

/// Installed assets are local-only: the mounted table resolves the pieces of a command's tooltip, the
/// group's key pairs, the cost line's keys and a build item's composition. Compares structure only and
/// commits no text. Set `STA64_TOOLTIP_TESTS=1 STA64_TEST_INSTALL` (and optionally `STA64_TEST_MOD`).
#[test]
fn installed_command_and_build_item_pieces_resolve() {
    use crate::ui::hud::stock::tooltip_text::*;
    if std::env::var("STA64_TOOLTIP_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_TOOLTIP_TESTS=1 STA64_TEST_INSTALL and optional STA64_TEST_MOD");
        return;
    }
    let install = std::env::var("STA64_TEST_INSTALL").expect("STA64_TEST_INSTALL");
    let active_mod = std::env::var("STA64_TEST_MOD").ok().filter(|s| !s.is_empty());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref()).unwrap();
    let strings = crate::ui::i18n::Strings::load(&vfs, "english");
    let look = |k: &str| strings.tooltip_text(k).to_owned();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let load = |stem: &str| sta_odf::resolve_layered(stem, &mut |n, v| index.load(&vfs, n, v)).ok();

    // One command: its short and verbose keys resolve, the marker is consumed and the letter in the
    // brackets is the bound one.
    let stop = load("stop").expect("the installed stop command");
    for key in ["tooltip", "verbose"] {
        let k = stop.str(key).expect("command key").to_owned();
        let text = look(&k);
        assert_ne!(text, k, "{key} resolves");
        let (shown, marks) = hotkey_text(&text, Some('x'));
        assert!(!shown.contains(HOTKEY_MARK), "the marker is consumed");
        eprintln!("command {key}: {} marked glyph(s), slot filled: {}", marks.len(), shown.contains("<X>"));
        assert!(!marks.is_empty() && shown.contains("<X>"), "the name and the slot are marked and the bound letter is in the slot");
    }

    // The group's key pairs.
    for (short, verbose) in [
        ("GUI_BP_MAIN_MENU", "GUI_CV_MAIN_TOOLTIP"),
        ("GUI_BP_COMM_MENU", "GUI_CV_COMM_TOOLTIP"),
        ("GUI_BP_IDLE_TOOLTIP", "GUI_BP_IDLE_VTOOLTIP"),
        ("GUI_BP_MORE_TOOLTIP", "GUI_BP_MORE_VTOOLTIP"),
    ] {
        assert!(look(short) != short && look(verbose) != verbose, "{short}");
    }

    // The cost line's pieces.
    for key in [COST_START_KEY, COST_SEPARATE_KEY, COST_END_KEY, "GUI_CP_DILITHIUM_RES", "GUI_CP_METAL_RES", "GUI_CP_CREW_RES", "GUI_CP_OFFICERS_RES"] {
        assert_ne!(look(key), key, "{key}");
    }

    // One build item: a class with a tooltip key, a hotkey label and a dilithium cost.
    let mut found = None;
    for (stem, _) in index.iter() {
        let Some(odf) = load(stem) else { continue };
        let key = |k: &str| odf.str(k).map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned);
        if let (Some(tooltip), Some(hotkey)) = (key("tooltip"), key("hotkeyLabel"))
            && odf.f32("dilithiumCost").is_some_and(|c| c > 0.0)
            && look(&tooltip) != tooltip
            && look(&hotkey) != hotkey
        {
            found = Some(BuildTipParts {
                tooltip,
                verbose: key("verboseTooltip"),
                hotkey: Some(hotkey),
                cost: vec![CostPart { key: "GUI_CP_DILITHIUM_RES", fallback: "dilithium".into(), amount: 7.0 }],
            });
            break;
        }
    }
    let p = found.expect("an installed class with tooltip, hotkeyLabel and a dilithium cost");
    let short = build_short(&p, &look);
    let verbose = build_verbose(&p, &look);
    let (open, close) = (look(COST_START_KEY), look(COST_END_KEY));
    let lines: Vec<&str> = short.lines().collect();
    assert_eq!(lines.len(), 2, "name line and cost line");
    assert!(lines[0].contains(&look(p.hotkey.as_deref().unwrap())), "the hotkey label follows the name");
    assert!(lines[1].starts_with(&open) && lines[1].ends_with(&close) && lines[1].contains(" 7 "), "the cost line is bracketed");
    assert_eq!(verbose.lines().next(), lines.first().copied(), "the verbose text starts with the same line");
    let full = look("GUI_CP_DILITHIUM_RES");
    assert!(verbose.lines().last().unwrap().contains(&full), "the verbose cost line has the full name");
    assert!(!lines[1].contains(&full) || full.chars().count() == 1, "the short one has only the first letter");
}

/// The box on a GPU frame: a flat colour of the layout key, no border, the size the rule gives.
/// `STA64_GPU_TESTS=1`; no game art (authored fonts, no glyph pictures).
#[test]
fn gpu_frame_of_the_box_is_flat_and_borderless() {
    use bevy::asset::RenderAssetUsages;
    use bevy::camera::RenderTarget;
    use bevy::render::{RenderPlugin, view::screenshot::{Screenshot, ScreenshotCaptured}};
    use bevy::window::ExitCondition;
    use std::sync::{Arc, Mutex};
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return;
    }
    let size = UVec2::new(800, 600);
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
    let (cfg, _) = sta_content::GuiCfg::parse("t.cfg", BG);
    let hud = size.as_vec2();
    app.insert_resource(UiProfile::Armada2)
        .insert_resource(fonts())
        .insert_resource(canvas(hud, hud, HudFit::Centred))
        .insert_resource(HudConfig { gui: Some(cfg.layout_auto()), ..default() })
        .add_plugins(NoticePlugin);
    let mut w = Window::default();
    w.resolution.set(hud.x, hud.y);
    w.set_cursor_position(Some(Vec2::new(250.0, 180.0)));
    app.world_mut().spawn((w, PrimaryWindow));
    let mut image = Image::new_target_texture(size.x, size.y, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    app.world_mut().spawn((Camera2d, bevy::ui::IsDefaultUiCamera, Camera { clear_color: ClearColorConfig::Custom(Color::srgb(0.2, 0.25, 0.3)), ..default() }, RenderTarget::from(target.clone()), Msaa::Off));
    app.finish();
    app.cleanup();
    for _ in 0..4 {
        app.update();
    }
    show(&mut app, "Hello");
    for _ in 0..6 {
        app.update();
    }
    let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    let output = Arc::clone(&frames);
    app.world_mut().spawn(Screenshot::image(target)).observe(move |shot: On<ScreenshotCaptured>| {
        output.lock().unwrap().push(shot.image.clone());
    });
    for _ in 0..90 {
        app.update();
        if !frames.lock().unwrap().is_empty() {
            break;
        }
    }
    let image = frames.lock().unwrap().first().cloned().expect("GPU capture");
    let rgba = image.try_into_dynamic().expect("GPU image").to_rgba8();
    let near = |x: u32, y: u32, want: [f32; 3]| {
        let p = rgba.get_pixel(x, y);
        (0..3).all(|i| (f32::from(p[i]) - want[i] * 255.0).abs() <= 2.0)
    };
    let fill = [0.25, 0.5, 0.75];
    let clear = [0.2, 0.25, 0.3];
    // The box is 56 x 18 px at (247, 208) (the placement rule at this window; authored 10 px glyphs).
    let (l, t, r, b) = (247u32, 208u32, 247 + 56, 208 + 18);
    for (x, y) in [(l, t), (r - 1, t), (l, b - 1), (r - 1, b - 1), (l + 2, t + 9), (l + 28, t + 9)] {
        assert!(near(x, y, fill), "({x},{y}) is not the fill: {:?}", rgba.get_pixel(x, y));
    }
    // No border: the first column and row are the fill too, and just outside is the clear colour.
    for (x, y) in [(l - 1, t + 9), (r, t + 9), (l + 28, t - 1), (l + 28, b)] {
        assert!(near(x, y, clear), "({x},{y}) is not clear: {:?}", rgba.get_pixel(x, y));
    }
    let flat = (t..b).flat_map(|y| (l..r).map(move |x| (x, y))).filter(|&(x, y)| near(x, y, fill)).count();
    assert_eq!(flat, 56 * 18, "the box is one flat colour");
}
