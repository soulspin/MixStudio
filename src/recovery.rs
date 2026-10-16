//! Atomic project writes and independent recovery snapshots.
use crate::model::Mix;
use serde::{Deserialize,Serialize};
use std::{fs,io::{self,Write},path::{Path,PathBuf},sync::atomic::{AtomicU64,Ordering}};

const MAX_RECORD_BYTES:usize=64*1024*1024;

pub fn unique_id()->String {
    static NEXT:AtomicU64=AtomicU64::new(0);
    format!("{}-{}-{}",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos(),std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed))
}
pub fn now()->u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() }
fn json_error(e:impl std::fmt::Display)->io::Error { io::Error::new(io::ErrorKind::InvalidData,e.to_string()) }
pub fn backup_path(path:&Path)->PathBuf { let mut p=path.as_os_str().to_os_string(); p.push(".bak"); PathBuf::from(p) }
/// Write and fsync a sibling temporary file before replacing the destination.
/// Any error before rename leaves the destination intact.
fn atomic_write(path:&Path,bytes:&[u8])->io::Result<()> {
    let parent=path.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let tmp=parent.join(format!(".mixstudio-{}.tmp",unique_id()));
    let result=(|| {
        let mut options=fs::OpenOptions::new(); options.write(true).create_new(true);
        #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
        let mut file=options.open(&tmp)?;
        if let Ok(meta)=fs::metadata(path) { file.set_permissions(meta.permissions())?; }
        file.write_all(bytes)?; file.sync_all()?; drop(file); fs::rename(&tmp,path)?;
        if let Ok(dir)=fs::File::open(parent) { let _=dir.sync_all(); }
        Ok(())
    })();
    if result.is_err() { let _=fs::remove_file(&tmp); } result
}
pub fn save_project(path:&Path,mix:&Mix)->io::Result<()> {
    crate::validation::validate_mix(mix).map_err(json_error)?;
    let bytes=serde_json::to_vec_pretty(mix).map_err(json_error)?;
    let _:Mix=serde_json::from_slice(&bytes).map_err(json_error)?;
    match fs::read(path) { Ok(old)=>atomic_write(&backup_path(path),&old)?,Err(e) if e.kind()==io::ErrorKind::NotFound=>{},Err(e)=>return Err(e) }
    atomic_write(path,&bytes)
}
#[derive(Clone,Serialize,Deserialize)]
pub struct Record { pub version:u32, pub id:String, pub saved_at:u64, pub original:Option<PathBuf>, pub mix:Mix }
impl Record { pub fn new(original:Option<PathBuf>,mix:Mix)->Self {Self{version:1,id:unique_id(),saved_at:now(),original,mix}} }
#[derive(Clone)]
pub struct Entry { pub path:PathBuf, pub record:Record }
pub fn write_record(dir:&Path,r:&Record)->io::Result<PathBuf> {
    crate::validation::validate_mix(&r.mix).map_err(json_error)?;
    if r.id.is_empty() || !r.id.bytes().all(|c|c.is_ascii_alphanumeric()||c==b'-') { return Err(json_error("Invalid recovery id")); }
    fs::create_dir_all(dir)?; let path=dir.join(format!("{}.recovery.json",r.id));
    let bytes=serde_json::to_vec(r).map_err(json_error)?;
    if bytes.len()>MAX_RECORD_BYTES {return Err(json_error("Recovery exceeds 64 MB; save the project manually before switching or closing."));}
    let _:Record=serde_json::from_slice(&bytes).map_err(json_error)?;
    atomic_write(&path,&bytes)?; Ok(path)
}
pub fn read_record(path:&Path)->io::Result<Record> {
    if fs::metadata(path)?.len()>MAX_RECORD_BYTES as u64 {return Err(json_error("Recovery file exceeds 64 MB"));}
    let r:Record=serde_json::from_slice(&fs::read(path)?).map_err(json_error)?;
    if r.version!=1 {return Err(json_error("Unsupported recovery version"));}
    crate::validation::validate_mix(&r.mix).map_err(json_error)?; Ok(r)
}
pub fn scan(dir:&Path)->(Vec<Entry>,usize) {
    let mut entries=vec![];let mut bad=0;
    match fs::read_dir(dir) {
        Ok(files)=>for file in files { match file {
            Ok(f) if f.file_type().is_ok_and(|t|t.is_file()) && f.file_name().to_string_lossy().ends_with(".recovery.json")=>match read_record(&f.path()) {Ok(record)=>entries.push(Entry{path:f.path(),record}),Err(_)=>bad+=1},
            Ok(_)=>{},Err(_)=>bad+=1,
        } },
        Err(e) if e.kind()==io::ErrorKind::NotFound=>{},Err(_)=>bad+=1,
    }
    entries.sort_by(|a,b|b.record.saved_at.cmp(&a.record.saved_at)); (entries,bad)
}
/// Relink every arranged/split occurrence without replacing analysis or source edit metadata.
pub fn relink(mix:&mut Mix,old:&str,new:&str)->Result<usize,String> {
    if old==new || new.is_empty() {return Err("Choose a different file path.".into());}
    if mix.phrase_edit.get(old).zip(mix.phrase_edit.get(new)).is_some_and(|(a,b)|a!=b) || mix.cue_marks.get(old).zip(mix.cue_marks.get(new)).is_some_and(|(a,b)|a!=b) {return Err("That source already has different cue or phrase edits. Choose another copy.".into());}
    let n=mix.tracks.iter().filter(|t|t.path==old).count(); if n==0 {return Err("The missing source is no longer in this project.".into());}
    for t in &mut mix.tracks {if t.path==old {t.path=new.into();}}
    if let Some(v)=mix.phrase_edit.remove(old) {mix.phrase_edit.insert(new.into(),v);}
    if let Some(v)=mix.cue_marks.remove(old) {mix.cue_marks.insert(new.into(),v);}
    Ok(n)
}
pub enum Job { Protect(Record,String), Write(Record), Remove(PathBuf), Scan, Read(PathBuf), Flush(crossbeam_channel::Sender<()>), Stop }
pub enum Event { Protected(String,Result<(),String>), Listed(Vec<Entry>,usize), Written(String,u64), Restored(PathBuf,Record), Error(String) }
pub struct Worker { pub tx:crossbeam_channel::Sender<Job>, pub rx:crossbeam_channel::Receiver<Event> }
impl Worker {
    pub fn new(dir:PathBuf)->Self {
        let (tx,jobs)=crossbeam_channel::unbounded(); let (events,rx)=crossbeam_channel::unbounded();
        std::thread::spawn(move || for job in jobs { let result=match job {
            Job::Protect(r,token)=>Ok(Event::Protected(token,write_record(&dir,&r).map(|_|()).map_err(|e|e.to_string()))),
            Job::Write(r)=>write_record(&dir,&r).map(|_|Event::Written(r.id,r.saved_at)),
            Job::Remove(path)=> { if path.parent()!=Some(dir.as_path()) {Err(json_error("Invalid recovery path"))} else {match fs::remove_file(path) {Ok(())=>Ok(Event::Written(String::new(),0)),Err(e) if e.kind()==io::ErrorKind::NotFound=>Ok(Event::Written(String::new(),0)),Err(e)=>Err(e)}} },
            Job::Scan=>{let (list,bad)=scan(&dir);Ok(Event::Listed(list,bad))},
            Job::Read(path)=>if path.parent()==Some(dir.as_path()) {read_record(&path).map(|r|Event::Restored(path,r))} else {Err(json_error("Invalid recovery path"))},
            Job::Flush(done)=>{let _=done.send(());continue;},Job::Stop=>break,
        }; let event=result.unwrap_or_else(|e|Event::Error(e.to_string())); let _=events.send(event); });
        Self{tx,rx}
    }
    pub fn flush(&self) {let (tx,rx)=crossbeam_channel::bounded(1);let _=self.tx.send(Job::Flush(tx));let _=rx.recv_timeout(std::time::Duration::from_secs(2));}
}
impl Drop for Worker {fn drop(&mut self){let _=self.tx.send(Job::Stop);}}

#[cfg(test)]
mod tests {
    use super::*;
    fn dir()->std::path::PathBuf { let p=std::env::temp_dir().join(format!("mixstudio-recovery-{}",unique_id())); std::fs::create_dir_all(&p).unwrap(); p }
    #[test]
    fn atomic_save_keeps_previous_version_and_failed_save_preserves_original() {
        let d=dir(); let path=d.join("mix.json"); let mut mix=crate::model::Mix::default();
        save_project(&path,&mix).unwrap(); let old=std::fs::read(&path).unwrap(); mix.name="Changed".into();
        save_project(&path,&mix).unwrap(); assert_eq!(std::fs::read(backup_path(&path)).unwrap(),old);
        let current=std::fs::read(&path).unwrap(); std::fs::remove_file(backup_path(&path)).unwrap(); std::fs::create_dir(backup_path(&path)).unwrap();
        mix.name="Must not replace original".into(); assert!(save_project(&path,&mix).is_err()); assert_eq!(std::fs::read(&path).unwrap(),current);
        std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn semantically_invalid_recovery_and_project_writes_are_rejected() {
        let d=dir(); let target=d.join("mix.json"); let good=Mix::default(); save_project(&target,&good).unwrap();
        let before=fs::read(&target).unwrap(); let mut bad=good.clone(); bad.bpm=0.0;
        assert!(save_project(&target,&bad).is_err(),"zero tempo was saved");
        assert_eq!(fs::read(&target).unwrap(),before);
        let record=Record::new(None,bad); let path=d.join("bad.recovery.json");
        fs::write(&path,serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(read_record(&path).is_err(),"invalid recovery was accepted");
        std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn recovery_is_independent_and_corrupt_or_unknown_versions_do_not_hide_valid_entries() {
        let d=dir(); let mix=crate::model::Mix::default(); let original=d.join("original.json"); save_project(&original,&mix).unwrap();
        let mut edited=mix.clone(); edited.name="Unsaved edits".into(); let record=Record::new(Some(original.clone()),edited);
        let path=write_record(&d,&record).unwrap(); assert_eq!(read_record(&path).unwrap().mix.name,"Unsaved edits");
        assert_eq!(serde_json::from_slice::<crate::model::Mix>(&std::fs::read(&original).unwrap()).unwrap().name,mix.name);
        std::fs::write(d.join("bad.recovery.json"),b"{bad").unwrap();
        let mut future=record.clone(); future.id=unique_id(); future.version=999; write_record(&d,&future).unwrap();
        let (entries,bad)=scan(&d); assert_eq!(entries.len(),1); assert_eq!(bad,2);
        std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn oversized_snapshot_cannot_replace_a_readable_recovery() {
        let d=dir();let mut r=Record::new(None,crate::model::Mix::default());let path=write_record(&d,&r).unwrap();let before=std::fs::read(&path).unwrap();
        r.mix.name="x".repeat(MAX_RECORD_BYTES);assert!(write_record(&d,&r).is_err());assert_eq!(std::fs::read(&path).unwrap(),before);assert!(read_record(&path).is_ok());
        std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn invalid_snapshot_cannot_replace_a_valid_recovery() {
        let d=dir();let mut r=Record::new(None,crate::model::Mix::default());let path=write_record(&d,&r).unwrap();let before=std::fs::read(&path).unwrap();
        r.mix.bpm=f64::NAN;assert!(write_record(&d,&r).is_err());assert_eq!(std::fs::read(&path).unwrap(),before);
        std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn relink_preserves_all_clips_timing_and_source_metadata_and_rejects_collisions() {
        let mut mix:crate::model::Mix=serde_json::from_str(include_str!("../tests/fixtures/unstable-flex-pair.json")).unwrap();
        let old=mix.tracks[0].path.clone(); let mut clip=mix.tracks[0].clone(); clip.sections=vec![(0.0,4.0),(8.0,12.0)]; mix.tracks.push(clip);
        mix.phrase_edit.insert(old.clone(),vec![(2.0,1)]); mix.cue_marks.insert(old.clone(),vec![(1.0,"Cue".into())]);
        let before=mix.clone(); assert_eq!(relink(&mut mix,&old,"/moved/audio.wav").unwrap(),2);
        for (a,b) in before.tracks.iter().zip(&mix.tracks) { let mut a=a.clone(); if a.path==old {a.path="/moved/audio.wav".into();} assert_eq!(serde_json::to_string(&a).unwrap(),serde_json::to_string(b).unwrap()); }
        assert_eq!(mix.cue_marks["/moved/audio.wav"],before.cue_marks[&old]); assert_eq!(mix.phrase_edit["/moved/audio.wav"],before.phrase_edit[&old]);
        let mut bad=before; bad.cue_marks.insert("/moved/audio.wav".into(),vec![(2.0,"Other".into())]); let serialized=serde_json::to_string(&bad).unwrap();
        assert!(relink(&mut bad,&old,"/moved/audio.wav").is_err()); assert_eq!(serde_json::to_string(&bad).unwrap(),serialized);
    }
}
