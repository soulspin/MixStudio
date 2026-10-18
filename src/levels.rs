//! Sample-level gain, soft clipping and explicitly non-LUFS source statistics.
use serde::{Serialize,Deserialize};
#[derive(Clone,Copy,Serialize,Deserialize,PartialEq,Debug)]
pub struct OutputSettings {pub enabled:bool,pub ceiling_db:f32}
impl Default for OutputSettings {fn default()->Self {Self{enabled:true,ceiling_db:0.0}}}
pub fn amplitude(db:f32)->f32 {10.0_f32.powf(db/20.0)}
pub fn dbfs(value:f32)->String {if value>0.0 {format!("{:.1} dBFS",20.0*value.log10())}else{"−inf dBFS".into()}}
impl OutputSettings {
    pub fn prepare(self)->OutputStage {OutputStage{enabled:self.enabled,ceiling:amplitude(self.ceiling_db)}}
}
#[derive(Clone,Copy)]
pub struct OutputStage {enabled:bool,ceiling:f32}
impl OutputStage {
    pub fn process(self,x:f32)->f32 {
        if !x.is_finite() {return 0.0;}
        if !self.enabled {return x;}
        let ceiling=self.ceiling;let v=x/ceiling;
        let y=if v.abs()<0.9 {v}else{v.signum()*(0.9+0.1*((v.abs()-0.9)*10.0).tanh())};
        y*ceiling
    }
}
#[derive(Clone,Copy,Debug)]
pub struct OutputLevels {pub pre_peak:f32,pub post_peak:f32,pub min_ratio:f32}
impl Default for OutputLevels {fn default()->Self {Self{pre_peak:0.0,post_peak:0.0,min_ratio:1.0}}}
impl OutputLevels {
    pub fn observe(&mut self,x:f32,y:f32) {
        self.pre_peak=self.pre_peak.max(x.abs());self.post_peak=self.post_peak.max(y.abs());
        if x.abs()>1e-20 {self.min_ratio=self.min_ratio.min((y/x).abs().clamp(0.0,1.0));}
    }
}
#[derive(Clone,Copy,Debug)]
pub struct SourceLevels {pub peak:f32,pub rms:f32}
impl SourceLevels {
    pub fn measure(audio:&crate::analysis::Audio)->Result<Self,String> {
        if audio.l.is_empty() || audio.l.len()!=audio.r.len() {return Err("No complete stereo source to measure".into());}
        let mut peak=0.0_f32;let mut sum=0.0_f64;
        for &v in audio.l.iter().chain(&audio.r) {
            if !v.is_finite() {return Err("Source contains non-finite samples".into());}
            peak=peak.max(v.abs());sum+=(v as f64).powi(2);
        }
        Ok(Self{peak,rms:(sum/(audio.l.len()*2) as f64).sqrt() as f32})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_soft_clipping_retains_the_legacy_transfer_exactly() {
        for k in -3000..=3000 {let x=k as f32/1000.0;let legacy=if x.abs()<0.9 {x}else{x.signum()*(0.9+0.1*((x.abs()-0.9)*10.0).tanh())};assert_eq!(OutputSettings::default().prepare().process(x).to_bits(),legacy.to_bits());}
    }
    #[test]
    fn stereo_source_statistics_include_silence_and_both_channels() {
        let a=crate::analysis::Audio{sr:8000,l:vec![0.0,1.0,0.0,-1.0],r:vec![0.0;4]};let stats=SourceLevels::measure(&a).unwrap();assert_eq!(stats.peak,1.0);assert_eq!(stats.rms,0.5);
        let a=crate::analysis::Audio{sr:8000,l:vec![0.0;8],r:vec![0.0;8]};let stats=SourceLevels::measure(&a).unwrap();assert_eq!(stats.peak,0.0);assert_eq!(stats.rms,0.0);
    }
}
