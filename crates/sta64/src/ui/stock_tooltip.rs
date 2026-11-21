//! The Armada II stock tooltip: the box, where it sits and when it shows (issue 1045).
//! Applies to the Armada II stock look only (profile [`UiProfile::Armada2`] with the stock font
//! rule available, as the other stock text); STA64's own HUD, Fleet Operations, Roots, Armada I
//! and every other look keep the box and delays of [`notices`](super::notices).
//!
//! Rules (grades per item; "static" is read from the Armada II executable):
//! - The box is a plain opaque rectangle in the layout's `tooltipBackgroundColor` with the text in
//!   `tooltipTextColor` (read from data), with no border and no shadow (static; measured on the
//!   planet frame: one flat colour, no edge pixels of another).
//! - The text is the default stock font role ([`StockRole::Default`]), top-left, wrapped by words at
//!   [`TIP_WRAP_UNITS`] (explicit line breaks kept), never clipped (static; measured: widest line
//!   under the limit on one frame).
//! - All sizes are in the HUD's virtual units: the 1600x1200 viewport the HUD is laid out in
//!   ([`STOCK_SPACE`]), one unit being `hud width / 1600` physical px across and `hud height / 1200`
//!   down. The box is the text block plus [`TIP_PAD_X`] on both sides, [`TIP_PAD_TOP`] above and
//!   [`TIP_PAD_BOTTOM`] below (static; box size measured on every frame at 800x600).
//! - Position, from the pointer in virtual units (static; measured on four frames at 800x600 and one
//!   at 1280x720): the text's left is the pointer's x snapped to the pixel grid, at least [`TIP_EDGE`]
//!   from the left, and pulled left so the box ends [`TIP_PAD_X`] short of the viewport's right edge
//!   when it would cross it; the text's top is the pointer's y plus [`TIP_BELOW`], or above the
//!   pointer (`y - (height + TIP_FLIP_GAP)`) when `y + height + TIP_FLIP_MARGIN` reaches the
//!   viewport's bottom; snapped, and not above the top. The flip, the top clamp and the left clamp
//!   were not seen in any frame (static only).
//! - The viewport the clamps use is the HUD's virtual viewport, not the window: on a centred wide HUD
//!   the tooltip stays inside the HUD area. Interim: the rule is read from the executable and fits
//!   frames at two sizes, but no frame reached an edge on a wide window.
//! - The box follows the pointer while it is shown and is drawn above the HUD panels (static; one
//!   frame shows it over the minimap).
//! - The drawn box sits [`TIP_BOX_SHIFT`] and the text pen [`TIP_TEXT_SHIFT`] whole pixels off the
//!   rule's origin (grades on the constants: the text's is the engine's quad shift, the box's is
//!   measured at one size with its cause not determined).
//! - One gate ([`stock_tip`]) serves the box, the delays and the world text: the Armada II look with
//!   the stock font rule (`SpriteFonts::stock_family`) and a stock HUD canvas. Fleet Operations 3.x,
//!   Dominion War and any other look that shares the Armada II look but not the font rule keep today's
//!   box, delays and name-only world tooltip (not examined).
//!
//! Delays: `tooltipDelay` and `verboseTooltipDelay` of the component definition file
//! (`odf/other/component.odf`), the defaults [`TipDelays::STOCK`] when a key is absent. Keys read from
//! data; defaults read from the executable, static, and measured once (short after about half a
//! second, verbose after about three). Both count from the start of the hover; the verbose text
//! replaces the short one; a target without verbose text keeps its short text. One real-time clock
//! serves every kind of target: interim, the original has two clocks that agree at normal game speed.
//!
//! Not done (see the PR): the fixed-frame and off modes of the profile's tooltip setting, dismissal on
//! click or key, behaviour while a button is held, the two clocks at other game speeds.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::notices::{TipBox, TipText, tip_style};
use super::profile::UiProfile;
use super::sprite_font::{Align, SpriteFonts, SpriteText, SpriteTextStyle};
use super::stock_font::{STOCK_SPACE, StockRole, StockTextCanvas};
use super::theme::Theme;
use crate::viewer::Mounted;

/// Widest line of the text, in virtual units. Static.
pub const TIP_WRAP_UNITS: f32 = 550.0;
/// Padding left and right of the text, in virtual units. Static, measured.
pub const TIP_PAD_X: f32 = 6.0;
/// Padding above the text, in virtual units. Static, measured.
pub const TIP_PAD_TOP: f32 = 2.0;
/// Padding below the text, in virtual units. Static, measured.
pub const TIP_PAD_BOTTOM: f32 = 6.0;
/// The text's top below the pointer, in virtual units. Static, measured.
pub const TIP_BELOW: f32 = 56.0;
/// Room the text and padding need below the pointer before the tooltip flips above, in virtual
/// units beyond the text's height. Static.
pub const TIP_FLIP_MARGIN: f32 = 64.0;
/// Distance of the flipped text's bottom from the pointer, in virtual units. Static.
pub const TIP_FLIP_GAP: f32 = 4.0;
/// Least distance of the text's left from the viewport's left edge, and of the text's right end from the
/// right edge when the right clamp applies, in virtual units. Static.
pub const TIP_EDGE: f32 = 12.0;
/// Whole pixels the drawn box sits below the rule's origin. Measured at one window size (800x600);
/// cause not determined; not confirmed on a native frame of another size.
pub const TIP_BOX_SHIFT: Vec2 = Vec2::new(0.0, 1.0);
/// Whole pixels the text pen sits right of and below the rule's origin: the engine's quad shift (the
/// pixel-centre convention of its renderer; read from the executable, static, measured). The stock HUD
/// text placement (`stock_font::place_offset`) deliberately drops it, since it places text relative to
/// its rectangle; the tooltip keeps it for the text-to-box offset the frames show.
pub const TIP_TEXT_SHIFT: Vec2 = Vec2::new(1.0, 1.0);

/// Where the tooltip goes, in virtual units of the HUD viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TipPlace {
    /// The text block's top left, on the pixel grid.
    pub text: Vec2,
    /// The box: the text block with its padding.
    pub rect: Rect,
}

/// The placement rule. `cursor` is the pointer in virtual units, `text` the text block's size in
/// virtual units and `per_unit` the physical pixels per virtual unit on each axis (which sets the
/// pixel grid the position snaps to).
pub fn tooltip_place(cursor: Vec2, text: Vec2, per_unit: Vec2) -> TipPlace {
    let snap = |v: f32, s: f32| (v * s + 0.5).floor() / s;
    let mut x = snap(cursor.x, per_unit.x);
    if x < TIP_EDGE {
        x = TIP_EDGE;
    } else if x + text.x + TIP_EDGE > STOCK_SPACE.x {
        x = STOCK_SPACE.x - TIP_EDGE - text.x;
    }
    let mut y = if cursor.y + text.y + TIP_FLIP_MARGIN < STOCK_SPACE.y { cursor.y + TIP_BELOW } else { cursor.y - (text.y + TIP_FLIP_GAP) };
    y = snap(y, per_unit.y);
    if y < 0.0 {
        y = 0.0;
    }
    let min = Vec2::new(x - TIP_PAD_X, y - TIP_PAD_TOP);
    let max = Vec2::new(x + text.x + TIP_PAD_X, y + text.y + TIP_PAD_BOTTOM);
    TipPlace { text: Vec2::new(x, y), rect: Rect { min, max } }
}

/// The tooltip in physical pixels of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TipPx {
    /// The box, edges on whole pixels.
    pub rect: Rect,
    /// The text block's top left.
    pub pen: Vec2,
}

/// [`TipPlace`] on the screen: `block` is the text block's size in physical px, `per_unit` as in
/// [`tooltip_place`], `origin` the HUD's top left in physical px.
pub fn physical_box(place: &TipPlace, block: Vec2, per_unit: Vec2, origin: Vec2) -> TipPx {
    let t = (origin + place.text * per_unit).round();
    let left = (t.x - TIP_PAD_X * per_unit.x).round();
    let right = (t.x + block.x + TIP_PAD_X * per_unit.x).round();
    let top = (t.y - TIP_PAD_TOP * per_unit.y).round() + TIP_BOX_SHIFT.y;
    let bottom = (t.y + block.y + TIP_PAD_BOTTOM * per_unit.y).round() + TIP_BOX_SHIFT.y;
    TipPx { rect: Rect { min: Vec2::new(left + TIP_BOX_SHIFT.x, top), max: Vec2::new(right + TIP_BOX_SHIFT.x, bottom) }, pen: t + TIP_TEXT_SHIFT }
}

/// The text block `text` makes in the stock default font at this HUD, in physical px, wrapped at
/// [`TIP_WRAP_UNITS`]. `None` when the stock font rule does not apply or the font lacks a character
/// of the text (the plain path then draws it, as for other stock text).
pub fn measure(fonts: &SpriteFonts, canvas: &StockTextCanvas, px: f32, text: &str) -> Option<Vec2> {
    let (font, _, scale) = fonts.stock_choice(StockRole::Default, canvas, px)?;
    if text.chars().any(|c| font.font.lacks(c)) {
        return None;
    }
    let hud = canvas.0?.hud;
    let lay = font.font.layout_block_xy(text, scale, true, Some(wrap_logical(hud) * px), Align::Left);
    Some(Vec2::new(lay.width, lay.height))
}

/// The wrap width in the HUD's layout units (what the text style carries).
pub fn wrap_logical(hud: Vec2) -> f32 {
    TIP_WRAP_UNITS * hud.x / STOCK_SPACE.x
}

/// The one condition for the stock tooltip: the Armada II look, the stock font rule of the mounted
/// install and a stock HUD on screen. Everything else (STA64's own HUD, Fleet Operations, Dominion War,
/// other installs, no HUD) keeps the plain box, STA64's delays and the name-only unit tooltip.
pub fn stock_tip(profile: Option<&UiProfile>, fonts: Option<&SpriteFonts>, canvas: Option<&StockTextCanvas>) -> bool {
    profile == Some(&UiProfile::Armada2) && fonts.is_some_and(|f| f.stock_family) && canvas.is_some_and(|c| c.0.is_some())
}

/// Whether the box is laid out by the stock rule this frame (the plain placement then stands down).
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TipStock(pub bool);

/// Seconds of rest before the short and the verbose tooltip.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct TipDelays {
    pub short: f32,
    pub verbose: f32,
}

impl TipDelays {
    /// The stock look's delays when the component definition names none. Read from the
    /// executable, static; measured once.
    pub const STOCK: TipDelays = TipDelays { short: 0.5, verbose: 3.0 };
}

impl Default for TipDelays {
    /// STA64's own look keeps its own delays.
    fn default() -> Self {
        TipDelays { short: super::notices::TIP_SHORT_SECS, verbose: super::notices::TIP_VERBOSE_SECS }
    }
}

/// The delays the component definition gives; a key it lacks (or a value that is not a finite,
/// non-negative number) takes the stock default.
pub fn delays_from(component: Option<&sta_odf::Odf>) -> TipDelays {
    let read = |key: &str, default: f32| component.and_then(|o| o.f32(key)).filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(default);
    TipDelays { short: read("tooltipDelay", TipDelays::STOCK.short), verbose: read("verboseTooltipDelay", TipDelays::STOCK.verbose) }
}

/// The resolved component definition of a mounted install or mod, `None` without one.
pub fn read_component(vfs: &sta_vfs::Vfs) -> Option<sta_odf::Odf> {
    let (index, _) = sta_content::OdfIndex::build(vfs);
    sta_odf::resolve_layered("component", &mut |name, version| index.load(vfs, name, version)).ok()
}

/// Keep [`TipDelays`] in step with the mounted install: under the Armada II stock look the delays
/// of its component definition, else STA64's own.
pub fn load_tip_delays(
    vfs: Option<Res<Mounted>>,
    profile: Option<Res<UiProfile>>,
    fonts: Option<Res<SpriteFonts>>,
    canvas: Option<Res<StockTextCanvas>>,
    mut delays: ResMut<TipDelays>,
) {
    let changed = vfs.as_ref().is_some_and(|v| v.is_changed())
        || profile.as_ref().is_some_and(|p| p.is_changed())
        || fonts.as_ref().is_some_and(|f| f.is_changed())
        || canvas.as_ref().is_some_and(|c| c.is_changed());
    if !changed {
        return;
    }
    let want = if stock_tip(profile.as_deref(), fonts.as_deref(), canvas.as_deref()) {
        delays_from(vfs.as_deref().and_then(|v| read_component(&v.0)).as_ref())
    } else {
        TipDelays::default()
    };
    if *delays != want {
        *delays = want;
    }
}

fn set<T: PartialEq>(slot: &mut Mut<T>, value: T) {
    if **slot != value {
        **slot = value;
    }
}

/// Lay the shown tooltip out by the stock rule, or hand it back to the plain placement.
#[allow(clippy::too_many_arguments)]
pub fn stock_tip_box(
    canvas: Option<Res<StockTextCanvas>>,
    fonts: Option<Res<SpriteFonts>>,
    profile: Option<Res<UiProfile>>,
    theme: Res<Theme>,
    ui_scale: Option<Res<UiScale>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut active: ResMut<TipStock>,
    mut boxed: Single<&mut Node, With<TipBox>>,
    mut words: Single<(&SpriteText, &mut SpriteTextStyle), With<TipText>>,
) {
    if boxed.display == Display::None {
        return;
    }
    let window = window.single().ok();
    let scale_factor = window.map_or(1.0, |w| w.scale_factor());
    let px = (ui_scale.map_or(1.0, |u| u.0) * scale_factor).max(0.1);
    let stock = stock_tip(profile.as_deref(), fonts.as_deref(), canvas.as_deref());
    let canvas = canvas.as_deref().filter(|_| stock);
    let block = canvas.zip(fonts.as_deref()).and_then(|(c, f)| measure(f, c, px, &words.0.text));
    let (Some(canvas), Some(block)) = (canvas.and_then(|c| c.0), block) else {
        if active.0 {
            active.0 = false;
            let mut node = Node::clone(&boxed);
            node.width = Val::Auto;
            node.height = Val::Auto;
            node.padding = UiRect::axes(Val::Px(theme.pad), Val::Px(theme.gap * 0.5));
            node.border = UiRect::left(Val::Px(4.0));
            node.max_width = Val::Px(theme.text_size * 20.0);
            set(&mut *boxed, node);
        }
        set(&mut words.1, tip_style(&theme));
        return;
    };
    if !active.0 {
        active.0 = true;
    }
    let style = SpriteTextStyle { tint_all: true, ..SpriteTextStyle::default().stock_role(StockRole::Default).wrapped(wrap_logical(canvas.hud)) };
    set(&mut words.1, style);
    let Some(cursor) = window.and_then(Window::cursor_position) else { return };
    let per_unit = canvas.hud * px / STOCK_SPACE;
    let origin = canvas.origin() * px;
    let cursor_units = (cursor * scale_factor - origin) / per_unit;
    let place = tooltip_place(cursor_units, block / per_unit, per_unit);
    let at = physical_box(&place, block, per_unit, origin);
    let logical = |v: f32| Val::Px(v / px);
    let size = at.rect.size();
    let mut node = Node::clone(&boxed);
    node.left = logical(at.rect.min.x);
    node.top = logical(at.rect.min.y);
    node.right = Val::Auto;
    node.width = logical(size.x);
    node.height = logical(size.y);
    node.padding = UiRect { left: logical(at.pen.x - at.rect.min.x), top: logical(at.pen.y - at.rect.min.y), right: Val::Px(0.0), bottom: Val::Px(0.0) };
    node.border = UiRect::ZERO;
    node.max_width = Val::Auto;
    set(&mut *boxed, node);
}
