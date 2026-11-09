//! `parity <refit-out> <ours-out>`: Refit's run against ours, objects by name
//! and marks in sim seconds (see [`crate::parity`]). Exit 1 on differences.

use crate::parity::{self, Source, Tolerances};

use super::{Fail, Opts, Outcome};

pub(crate) fn parity(o: &Opts) -> Outcome {
    if o.names {
        print!("{}", parity::names_table());
        return Ok(0);
    }
    let [a, b] = o.rest.as_slice() else {
        return Err(Fail::Usage(
            "parity <refit-out> <ours-out> [--tol time=0.1,pos=1,hull=1] | --names".into(),
        ));
    };
    let tol = match &o.tol {
        Some(t) => Tolerances::parse(t).map_err(|e| Fail::Usage(format!("--tol: {e}")))?,
        None => Tolerances::default(),
    };
    let ra = parity::load(a.as_ref(), Source::Refit).map_err(Fail::Error)?;
    let rb = parity::load(b.as_ref(), Source::Ours).map_err(Fail::Error)?;
    let d = parity::diff(&ra, &rb, &tol);
    print!("{}", d.report(&format!("{a} (Refit)"), &format!("{b} (ours)")));
    Ok(i32::from(!d.is_empty()))
}
