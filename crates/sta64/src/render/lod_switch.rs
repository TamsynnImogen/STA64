//! The one decision between an object's model and its symbol (RND-27).
//!
//! On the map an object is either its model or its 2D symbol, never both.
//! [`decide`] is the only place that says which, once per object per frame, and
//! `render::strategic` applies the answer to every path that draws the object: the
//! model root's visibility (and with it the selection ring and health bar that
//! are its children), the symbol's UI node and its selection outline, and clicks.
//! Nothing else reads a distance or a pixel size to choose.
//!
//! # Where the overlap came from
//!
//! Before #365's repair (docs/STRATEGIC_HANDOFF.md) there were two independent
//! decisions. The symbol faded in and out by projected radius, 14 px down to 7 px,
//! while the model root was hidden by a separate rule at a fixed camera height of
//! 2500 units that knew nothing of the object's size, and only for roots with a
//! `SimEntity`. So for a ship whose radius crossed 7..14 px at some other height
//! the faded symbol and the model were both on screen, and a static root with
//! strategic art never lost its model at all. That repair made the choice one
//! per-object decision with a hysteresis gap, which is the rule here. What it did
//! not cover, and this does: RND-07's culling fade (a model's fade from 0.9 of the
//! culling distance) was a third, independent reason for a model to vanish, and a
//! per-class threshold had no home. The fade band is now the hysteresis of the
//! cull handoff below, so a faded-out model is replaced by its symbol rather than
//! leaving neither, and the fade-in cannot overlap the symbol.
//!
//! # The rule
//!
//! In `Auto`, from the previous frame's choice:
//!
//! - model to symbol when the projected radius is at most `enter_px`, or the object
//!   is past the culling distance ([`CullParams::fade`] is 0);
//! - symbol to model when the projected radius is at least `return_px` *and* the
//!   object is inside the start of the fade band (fade 1).
//!
//! `enter_px` < `return_px` and the band's 0.9 give the gap that stops a flicker.
//! `Always` and `Off` (Tab) override. The pixel thresholds are STA64's own
//! presentation policy per object class ([`SwitchParams`]); the games define no
//! pixel size for it. The distances are the games' (`cfgOBJECT_CULLING_DISTANCE`,
//! see [`super::cull`]).

use bevy::prelude::*;

use super::cull::CullParams;
use super::strategic::{Mode, Shape};

/// What an object is drawn as.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Representation {
    #[default]
    Model,
    Symbol,
}

/// Object classes the thresholds are kept per.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Ship,
    Station,
    Support,
}

impl Class {
    pub fn of(shape: Shape) -> Class {
        match shape {
            Shape::Arrow => Class::Ship,
            Shape::Square => Class::Station,
            Shape::Circle => Class::Support,
        }
    }
}

/// Projected radii, in logical pixels, where a class changes representation.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Thresholds {
    /// At or below this the model becomes the symbol.
    pub enter_px: f32,
    /// At or above this the symbol becomes the model again.
    pub return_px: f32,
}

#[derive(Resource, Clone, Copy, PartialEq, Debug)]
pub struct SwitchParams {
    pub ship: Thresholds,
    pub station: Thresholds,
    pub support: Thresholds,
}

impl Default for SwitchParams {
    fn default() -> Self {
        let t = Thresholds { enter_px: super::strategic::SYMBOL_ENTER_PX, return_px: super::strategic::MODEL_RETURN_PX };
        SwitchParams { ship: t, station: t, support: t }
    }
}

impl SwitchParams {
    pub fn of(&self, class: Class) -> Thresholds {
        match class {
            Class::Ship => self.ship,
            Class::Station => self.station,
            Class::Support => self.support,
        }
    }
}

/// The representation of one object this frame.
///
/// `px` is its projected radius, `distance` its [`CullParams::distance`] from the
/// camera.
pub fn decide(mode: Mode, previous: Representation, px: f32, distance: f32, cull: &CullParams, t: Thresholds) -> Representation {
    match mode {
        Mode::Always => Representation::Symbol,
        Mode::Off => Representation::Model,
        Mode::Auto => {
            let fade = cull.fade(distance);
            match previous {
                Representation::Symbol if px >= t.return_px && fade >= 1.0 => Representation::Model,
                Representation::Symbol => Representation::Symbol,
                Representation::Model if px <= t.enter_px || fade <= 0.0 => Representation::Symbol,
                Representation::Model => Representation::Model,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Thresholds = Thresholds { enter_px: 7.0, return_px: 10.0 };

    fn step(prev: Representation, px: f32, d: f32) -> Representation {
        decide(Mode::Auto, prev, px, d, &CullParams::default(), T)
    }

    #[test]
    fn pixel_size_has_a_gap() {
        use Representation::*;
        assert_eq!(step(Model, 7.01, 100.0), Model);
        assert_eq!(step(Model, 7.0, 100.0), Symbol);
        assert_eq!(step(Symbol, 9.99, 100.0), Symbol);
        assert_eq!(step(Symbol, 10.0, 100.0), Model);
    }

    #[test]
    fn the_cull_band_is_the_hysteresis_of_the_distance_handoff() {
        use Representation::*;
        // Big on screen all the way: only the distance decides.
        assert_eq!(step(Model, 500.0, 2799.0), Model);
        assert_eq!(step(Model, 500.0, 2800.0), Symbol);
        // Back inside the culling distance but still in the band: stays a symbol.
        assert_eq!(step(Symbol, 500.0, 2700.0), Symbol);
        assert_eq!(step(Symbol, 500.0, 2520.0), Model);
        // A model in the band is not taken out of it.
        assert_eq!(step(Model, 500.0, 2700.0), Model);
    }

    #[test]
    fn overrides_win() {
        let c = CullParams::default();
        assert_eq!(decide(Mode::Always, Representation::Model, 900.0, 1.0, &c, T), Representation::Symbol);
        assert_eq!(decide(Mode::Off, Representation::Symbol, 0.0, 1.0e6, &c, T), Representation::Model);
    }

    #[test]
    fn classes_map_from_shapes_and_default_alike() {
        assert_eq!(Class::of(Shape::Arrow), Class::Ship);
        assert_eq!(Class::of(Shape::Square), Class::Station);
        assert_eq!(Class::of(Shape::Circle), Class::Support);
        let p = SwitchParams::default();
        assert_eq!(p.of(Class::Ship), T);
    }
}
