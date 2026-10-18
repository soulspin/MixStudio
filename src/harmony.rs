//! Explain whole-track Camelot labels without claiming measured overlap harmony.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Relation { Same, Neighbor, Relative, Different, Unknown }
impl Relation {
    pub fn between(a:&str,b:&str)->Self {
        let (Some((a,am)),Some((b,bm)))=(crate::automix::camelot(a),crate::automix::camelot(b)) else {return Self::Unknown;};
        if a==b && am==bm {Self::Same}
        else if am==bm && ((a-b).abs()==1 || (a-b).abs()==11) {Self::Neighbor}
        else if a==b {Self::Relative}
        else {Self::Different}
    }
    pub fn label(self)->&'static str {match self {Self::Same=>"Same key",Self::Neighbor=>"Neighboring keys",Self::Relative=>"Relative major / minor",Self::Different=>"Different keys",Self::Unknown=>"Unknown key"}}
    pub fn reason(self)->&'static str {match self {
        Self::Same=>"Same Camelot number and mode: a useful starting point for a melodic blend.",
        Self::Neighbor=>"One Camelot step apart in the same mode: closely related keys.",
        Self::Relative=>"Same Camelot number, opposite letters: relative major and minor share their scale notes.",
        Self::Different=>"Outside the usual same / neighboring / relative matches. A musical clash is possible, not certain.",
        Self::Unknown=>"At least one source has no recognized Camelot key. Compatibility cannot be assessed from these labels.",
    }}
    pub fn tip(self)->&'static str {match self {
        Self::Same|Self::Neighbor|Self::Relative=>"Try a melodic blend, but listen for competing bass notes or overlapping vocals.",
        Self::Different=>"Try a shorter or percussion-led overlap, or hand over the bass before the next melody arrives.",
        Self::Unknown=>"Verify the source keys by listening before relying on harmonic ordering.",
    }}
}
pub fn key_name(key:&str)->Option<&'static str> {
    let (n,mode)=crate::automix::camelot(key)?;
    Some(if mode=='A' { ["A-flat minor","E-flat minor","B-flat minor","F minor","C minor","G minor","D minor","A minor","E minor","B minor","F-sharp minor","C-sharp minor"] }
         else { ["B major","F-sharp major","D-flat major","A-flat major","E-flat major","B-flat major","F major","C major","G major","D major","A major","E major"] }[(n-1) as usize])
}
pub fn common_matches(key:&str)->Option<String> {
    let (n,m)=crate::automix::camelot(key)?;let other=if m=='A' {'B'}else{'A'};
    Some(format!("{n}{m}, {}{m}, {}{m}, {n}{other}",(n+10)%12+1,n%12+1))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relationships_cover_wraparound_relative_and_unknown_keys() {
        for (a,b,want) in [("8A","8A",Relation::Same),("12A","1A",Relation::Neighbor),("8a"," 9A ",Relation::Neighbor),("8A","8B",Relation::Relative),("8A","9B",Relation::Different),("8A","?",Relation::Unknown),("13A","1A",Relation::Unknown)] {assert_eq!(Relation::between(a,b),want);assert_eq!(Relation::between(b,a),want);}
        assert_eq!(key_name("8A"),Some("A minor"));assert_eq!(key_name("8B"),Some("C major"));assert_eq!(key_name("0B"),None);
        assert_eq!(common_matches("12A").unwrap(),"12A, 11A, 1A, 12B");
    }
}
