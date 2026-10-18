//! Shared clip-edge envelope for the timeline, realtime renderer and export.
#[inline]
pub fn value(progress:f64, shape:f32)->f64 {
    let shape=if shape.is_finite() {shape.clamp(-1.0,1.0)} else {0.0};
    if shape==0.0 {return progress.clamp(0.0,1.0);}
    progress.clamp(0.0,1.0).powf(2.0_f64.powf(2.0*shape as f64))
}
pub fn shape_from_midpoint(gain:f32)->f32 {
    ((gain.clamp(0.0625,0.8408964) as f64).ln()/0.5_f64.ln()).log2().mul_add(0.5,0.0).clamp(-1.0,1.0) as f32
}
