use crate::engine::{Cmd, Engine, Shared};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use std::sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}};

const BLOCK_FRAMES: usize = 512;
const BLOCK_COUNT: usize = 8;

struct AudioBlock {
    samples: [f32; BLOCK_FRAMES * 2], clock: [f64; BLOCK_FRAMES],
    end_beat: f64, epoch: u64, meter: [u32; 2], deck_meter: [u32; 4],
}
impl AudioBlock {
    fn new() -> Self { Self { samples: [0.0; BLOCK_FRAMES*2], clock: [0.0; BLOCK_FRAMES], end_beat: 0.0, epoch: 0, meter: [0;2], deck_meter: [0;4] } }
}

// Even values are transport generations; the low bit protects public playback
// clock publication. The device callback tries once and never waits for the worker.
#[derive(Default)]
struct Transport { epoch: AtomicU64 }
impl Transport {
    fn change(&self, shared: &Shared, beat: Option<f64>, playing: Option<bool>) -> (u64, f64) {
        let epoch=loop {
            let epoch=self.epoch.load(Ordering::Acquire);
            if epoch&1==0 && self.epoch.compare_exchange(epoch,epoch|1,Ordering::Acquire,Ordering::Relaxed).is_ok() { break epoch; }
            std::thread::yield_now();
        };
        let position=beat.unwrap_or_else(||shared.beat());
        shared.beat.store(position.to_bits(),Ordering::Relaxed);
        if let Some(playing)=playing {
            shared.playing.store(playing,Ordering::Relaxed);
            if !playing {for meter in shared.meter.iter().chain(shared.deck_meter.iter()) {meter.store(0.0f32.to_bits(),Ordering::Relaxed);}}
        }
        self.epoch.store(epoch.wrapping_add(2),Ordering::Release);
        (epoch.wrapping_add(2),position)
    }
}

struct DeviceReader {
    ready: Receiver<Box<AudioBlock>>, returned: Sender<Box<AudioBlock>>,
    current: Option<Box<AudioBlock>>, pending_return: Option<Box<AudioBlock>>, position: usize,
    shared: Arc<Shared>, transport: Arc<Transport>, lifecycle: Option<(Arc<AtomicBool>,Arc<AtomicBool>)>,
}
impl Drop for DeviceReader {
    fn drop(&mut self) {
        if let Some((armed,stop))=&self.lifecycle {if armed.swap(false,Ordering::AcqRel) {stop.store(true,Ordering::Release);}}
        if let Some(block)=self.current.take() {let _=self.returned.try_send(block);}
        if let Some(block)=self.pending_return.take() {let _=self.returned.try_send(block);}
    }
}
impl DeviceReader {
    fn recycle(&mut self, block: Box<AudioBlock>) -> bool {
        match self.returned.try_send(block) {
            Ok(())=>true,
            Err(e)=>{ self.pending_return=Some(e.into_inner()); false }
        }
    }
    fn fill<T: Copy>(&mut self, output: &mut [T], channels: usize, convert: impl Fn(f32)->T) {
        output.fill(convert(0.0));
        if channels==0 { return; }
        let epoch=self.transport.epoch.load(Ordering::Acquire);
        if epoch&1!=0 || self.transport.epoch.compare_exchange(epoch,epoch|1,Ordering::Acquire,Ordering::Relaxed).is_err() { return; }
        self.fill_locked(output,channels,&convert,epoch);
        self.transport.epoch.store(epoch,Ordering::Release);
    }
    fn fill_locked<T: Copy>(&mut self, output: &mut [T], channels: usize, convert: &impl Fn(f32)->T, epoch: u64) {
        if let Some(block)=self.pending_return.take() { if !self.recycle(block) { return; } }
        if self.current.as_ref().is_some_and(|b|b.epoch!=epoch) {
            let block=self.current.take().unwrap(); if !self.recycle(block) { return; }
            self.position=0;
        }
        if !self.shared.playing.load(Ordering::Relaxed) { return; }
        for frame in output.chunks_exact_mut(channels) {
            if self.current.is_none() {
                // There are at most BLOCK_COUNT stale pooled blocks to discard.
                while let Ok(block)=self.ready.try_recv() {
                    if block.epoch==epoch { self.current=Some(block);self.position=0;break; }
                    if !self.recycle(block) { return; }
                }
            }
            let Some(block)=self.current.as_ref() else { self.shared.underruns.fetch_add(1,Ordering::Relaxed);return; };
            let sample=&block.samples[self.position*2..self.position*2+2];
            for (channel,value) in frame.iter_mut().enumerate() { *value=convert(sample[channel%2]); }
            self.position+=1;
            let beat=if self.position==BLOCK_FRAMES {block.end_beat} else {block.clock[self.position]};
            self.shared.beat.store(beat.to_bits(),Ordering::Relaxed);
            if self.position==BLOCK_FRAMES {
                for (dst,value) in self.shared.meter.iter().zip(block.meter) {dst.store(value,Ordering::Relaxed);}
                for (dst,value) in self.shared.deck_meter.iter().zip(block.deck_meter) {dst.store(value,Ordering::Relaxed);}
                let block=self.current.take().unwrap();if !self.recycle(block) {return;}
            }
        }
    }
}

struct WorkerState {
    engine: Engine, internal: Arc<Shared>, shared: Arc<Shared>, transport: Arc<Transport>,
    epoch: u64, playing: bool, loop_range: Option<(f64,f64)>,
}
impl WorkerState {
    fn new(sr: f32, shared: Arc<Shared>, transport: Arc<Transport>) -> Self {
        let internal=Arc::new(Shared::new());
        Self {engine:Engine::new(sr,internal.clone()),internal,shared,transport,epoch:0,playing:false,loop_range:None}
    }
    fn handle(&mut self, command: Cmd) {
        match &command {
            Cmd::Seek(beat)=>{self.epoch=self.transport.change(&self.shared,Some(*beat),None).0;}
            Cmd::Play(playing)=>{
                if !*playing {
                    let (epoch,beat)=self.transport.change(&self.shared,None,Some(false));self.epoch=epoch;
                    self.engine.handle(Cmd::Seek(beat));
                } else {self.shared.playing.store(true,Ordering::Relaxed);}
                self.playing=*playing;
            }
            Cmd::Loop(range) if *range!=self.loop_range=>{
                let (epoch,beat)=self.transport.change(&self.shared,None,None);self.epoch=epoch;
                self.engine.handle(Cmd::Seek(beat));self.loop_range=*range;
            }
            _=>{}
        }
        self.engine.handle(command);
    }
    fn render(&mut self, block: &mut AudioBlock) {
        block.epoch=self.epoch;
        self.engine.render_with_clock(&mut block.samples,2,&mut block.clock);
        block.end_beat=self.internal.beat();
        for (dst,src) in block.meter.iter_mut().zip(self.internal.meter.iter()) {*dst=src.load(Ordering::Relaxed);}
        for (dst,src) in block.deck_meter.iter_mut().zip(self.internal.deck_meter.iter()) {*dst=src.load(Ordering::Relaxed);}
    }
}

// Keep the queues and public state alive until the stream closure is gone. Its
// final buffer/channel/Arc destruction consequently happens on Audio's owner.
struct WorkerGuard {
    stop: Arc<AtomicBool>, lifecycles: Vec<Arc<AtomicBool>>, thread: Option<std::thread::JoinHandle<()>>,
    ready: Receiver<Box<AudioBlock>>, returned: Sender<Box<AudioBlock>>,
    shared: Arc<Shared>, transport: Arc<Transport>,
}
impl WorkerGuard {
    fn start(sr: f32, commands: Receiver<Cmd>, shared: Arc<Shared>) -> std::io::Result<Self> {
        let (ready_tx,ready)=crossbeam_channel::bounded::<Box<AudioBlock>>(BLOCK_COUNT);
        let (returned,returned_rx)=crossbeam_channel::bounded(BLOCK_COUNT+1);
        for _ in 0..BLOCK_COUNT {returned.send(Box::new(AudioBlock::new())).unwrap();}
        let stop=Arc::new(AtomicBool::new(false));let transport=Arc::new(Transport::default());
        let worker_stop=stop.clone();let worker_shared=shared.clone();let worker_transport=transport.clone();
        let stale=ready.clone();let recycle=returned.clone();
        let thread=std::thread::Builder::new().name("mixstudio-audio".into()).spawn(move||{
            let mut worker=WorkerState::new(sr,worker_shared,worker_transport);
            while !worker_stop.load(Ordering::Acquire) {
                while let Ok(command)=commands.try_recv() {
                    let previous=worker.epoch;worker.handle(command);
                    if worker.epoch!=previous {while let Ok(block)=stale.try_recv() {let _=recycle.try_send(block);}}
                }
                if worker.playing {
                    if let Ok(mut block)=returned_rx.try_recv() {
                        worker.render(&mut block);
                        if ready_tx.send(block).is_err() {break;}
                        continue;
                    }
                }
                match commands.recv_timeout(std::time::Duration::from_millis(2)) {
                    Ok(command)=>{
                        let previous=worker.epoch;worker.handle(command);
                        if worker.epoch!=previous {while let Ok(block)=stale.try_recv() {let _=recycle.try_send(block);}}
                    },
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected)=>break,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout)=>{}
                }
            }
        })?;
        Ok(Self {stop,lifecycles:Vec::new(),thread:Some(thread),ready,returned,shared,transport})
    }
    fn reader(&mut self) -> DeviceReader {
        let armed=Arc::new(AtomicBool::new(false));self.lifecycles.push(armed.clone());
        DeviceReader {ready:self.ready.clone(),returned:self.returned.clone(),current:None,pending_return:None,position:0,shared:self.shared.clone(),transport:self.transport.clone(),lifecycle:Some((armed,self.stop.clone()))} }
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.stop.store(true,Ordering::Release);
        if let Some(thread)=self.thread.take() {let _=thread.join();}
    }
}

pub struct Audio { preparation: std::sync::Mutex<crate::engine::SnapshotBuilder>, sr: f32, pub tx: Sender<Cmd>, pub shared: Arc<Shared>, pub _stream: Option<cpal::Stream>, pub info: String, worker: Option<WorkerGuard> }
impl Drop for Audio {
    fn drop(&mut self) {drop(self._stream.take());drop(self.worker.take());}
}
impl Audio {
    pub fn snapshot(&self, snap: Arc<crate::engine::Snap>) -> Result<(), crossbeam_channel::SendError<Cmd>> {
        let mut builder=self.preparation.lock().unwrap_or_else(|e|e.into_inner());
        self.tx.send(Cmd::Prepared(builder.prepare(snap,self.sr)))
    }
    #[cfg(test)]
    pub fn test_output(tx: Sender<Cmd>, shared: Arc<Shared>) -> Self {
        Self {tx,shared,_stream:None,info:String::new(),sr:44100.0,preparation:Default::default(),worker:None}
    }
}
pub fn start() -> Audio {
    let (tx, rx) = crossbeam_channel::bounded::<Cmd>(1024);
    let shared = Arc::new(Shared::new());
    match build(rx, shared.clone()) {
        Ok((s, info, sr, worker)) => Audio { tx, shared, _stream: Some(s), info, sr:sr as f32, preparation:Default::default(),worker:Some(worker) },
        Err(e) => Audio { tx, shared, _stream: None, info: format!("No audio: {e}"), sr:44100.0, preparation:Default::default(),worker:None },
    }
}
fn build(rx: Receiver<Cmd>, shared: Arc<Shared>) -> anyhow::Result<(cpal::Stream, String, u32, WorkerGuard)> {
    let host = cpal::default_host();
    let dev = host.default_output_device().ok_or_else(|| anyhow::anyhow!("no audio device"))?;
    let def = dev.default_output_config()?;
    let sr = def.sample_rate().0;let ch = def.channels() as usize;
    let mut cfg: cpal::StreamConfig = def.config();cfg.buffer_size = cpal::BufferSize::Default;
    let name = dev.name().unwrap_or_default();
    let mut worker=WorkerGuard::start(sr as f32,rx,shared)?;
    let mut mk = |cfg: &cpal::StreamConfig| -> anyhow::Result<cpal::Stream> {
        let mut reader=worker.reader();let armed=reader.lifecycle.as_ref().unwrap().0.clone();let error_stop=worker.stop.clone();let error_shared=worker.shared.clone();
        let err=move |e| {error_stop.store(true,Ordering::Release);error_shared.playing.store(false,Ordering::Relaxed);eprintln!("audio error: {e}");};
        let stream=match def.sample_format() {
            cpal::SampleFormat::F32=>dev.build_output_stream(cfg,move |out:&mut[f32],_|reader.fill(out,ch,|x|x),err,None)?,
            cpal::SampleFormat::I16=>dev.build_output_stream(cfg,move |out:&mut[i16],_|reader.fill(out,ch,|x|(x*32767.0) as i16),err,None)?,
            f=>anyhow::bail!("audio format {f:?} not supported"),
        };
        armed.store(true,Ordering::Release);Ok(stream)
    };
    let stream=match mk(&cfg) {Ok(stream)=>stream,Err(_)=>mk(&def.config())?};
    if let Err(error)=stream.play() {drop(stream);return Err(error.into());}
    Ok((stream,format!("{name} · {sr} Hz"),sr,worker))
}

#[cfg(test)]
fn test_reader() -> (DeviceReader, Arc<Transport>, Sender<Box<AudioBlock>>, Receiver<Box<AudioBlock>>) {
    let (ready_tx,ready)=crossbeam_channel::bounded(BLOCK_COUNT);
    let (returned,returned_rx)=crossbeam_channel::bounded(BLOCK_COUNT+1);
    let transport=Arc::new(Transport::default());
    (DeviceReader {ready,returned,current:None,pending_return:None,position:0,shared:Arc::new(Shared::new()),transport:transport.clone(),lifecycle:None},transport,ready_tx,returned_rx)
}
#[cfg(test)]
mod device_diagnostics {
    use super::*;
    #[test]
    #[ignore = "requires local saved mix and active audio device; plays the transition"]
    fn saved_mix_device_clock_keeps_real_time() {
        let path=std::env::var("MIXSTUDIO_DIAGNOSTIC_PROJECT").unwrap();
        let mix:crate::model::Mix=serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let audio:Vec<_>=mix.tracks.iter().map(|t|Some(Arc::new(crate::analysis::load(std::path::Path::new(&t.path)).unwrap().0))).collect();
        let out=start(); println!("Device: {}",out.info); assert!(out._stream.is_some());
        let transitions=mix.transitions();
        let index=std::env::var("MIXSTUDIO_DIAGNOSTIC_TRANSITION").ok().and_then(|s|s.parse::<usize>().ok()).unwrap_or(0);
        let from=transitions[index].2+24.0;
        out.snapshot(Arc::new(crate::engine::Snap::build(&mix,&audio))).unwrap();
        out.tx.send(Cmd::Seek(from)).unwrap();out.tx.send(Cmd::Play(true)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let measured_from=out.shared.beat();let underruns_before=out.shared.underruns.load(Ordering::Relaxed);
        let begin=std::time::Instant::now();
        for _ in 0..250 { out.snapshot(Arc::new(crate::engine::Snap::build(&mix,&audio))).unwrap();std::thread::sleep(std::time::Duration::from_millis(16)); }
        let elapsed=begin.elapsed().as_secs_f64(); let progressed=mix.secs(measured_from,out.shared.beat());
        let underruns=out.shared.underruns.load(Ordering::Relaxed)-underruns_before;
        out.tx.send(Cmd::Play(false)).unwrap();
        println!("Device wall time {elapsed:.3}s, audio timeline progressed {progressed:.3}s, ratio {:.3}, callback underruns after warmup {underruns}",progressed/elapsed);
        assert!((progressed/elapsed-1.0).abs()<0.05,"audio device playback clock is running too slow or fast");
        assert_eq!(underruns,0,"audio worker starved the device callback after warmup");
    }
}

#[cfg(test)]
mod buffered_tests {
    use super::*;
    fn empty_snapshot() -> Arc<crate::engine::Snap> {
        Arc::new(crate::engine::Snap::build(&crate::model::Mix::default(),&[]))
    }
    #[test]
    fn worker_lookahead_does_not_publish_clock_and_snapshots_do_not_flush() {
        let shared=Arc::new(Shared::new());let transport=Arc::new(Transport::default());
        let mut worker=WorkerState::new(8000.0,shared.clone(),transport.clone());
        let mut builder=crate::engine::SnapshotBuilder::default();
        worker.handle(Cmd::Prepared(builder.prepare(empty_snapshot(),8000.0)));
        worker.handle(Cmd::Play(true));let mut block=AudioBlock::new();worker.render(&mut block);
        assert!(worker.internal.beat()>0.0);assert_eq!(shared.beat(),0.0);
        let epoch=worker.epoch;
        for _ in 0..10 {worker.handle(Cmd::Prepared(builder.prepare(empty_snapshot(),8000.0)));}
        assert_eq!(worker.epoch,epoch);assert_eq!(transport.epoch.load(Ordering::Relaxed),epoch);
    }
    #[test]
    fn worker_seek_pause_and_loop_restart_at_consumed_position() {
        let shared=Arc::new(Shared::new());let transport=Arc::new(Transport::default());
        let mut worker=WorkerState::new(8000.0,shared.clone(),transport);
        worker.handle(Cmd::Snap(empty_snapshot()));worker.handle(Cmd::Seek(4.0));
        worker.handle(Cmd::Play(true));let mut block=AudioBlock::new();worker.render(&mut block);
        assert!(worker.internal.beat()>4.0);assert_eq!(shared.beat(),4.0);
        shared.beat.store(4.01f64.to_bits(),Ordering::Relaxed);worker.handle(Cmd::Play(false));
        assert_eq!(worker.internal.beat(),4.01);assert!(!shared.playing.load(Ordering::Relaxed));
        worker.handle(Cmd::Play(true));worker.render(&mut block);
        worker.handle(Cmd::Loop(Some((4.0,4.02))));assert_eq!(worker.internal.beat(),4.01);
        let epoch=worker.epoch;worker.handle(Cmd::Loop(Some((4.0,4.02))));assert_eq!(worker.epoch,epoch);
        worker.render(&mut block);
        assert!(block.clock.iter().all(|&b|(4.0..4.02).contains(&b)));
        assert!((4.0..4.02).contains(&block.end_beat));
    }
    #[test]
    fn consumed_clock_uses_per_frame_loop_mapping_and_exact_block_end() {
        let (mut reader,_control,ready,returned)=test_reader();reader.shared.playing.store(true,Ordering::Relaxed);
        let mut block=Box::new(AudioBlock::new());block.samples.fill(0.2);
        block.clock=std::array::from_fn(|i|3.0+(i%4) as f64*0.01);block.end_beat=3.0;
        block.meter=[0.5f32.to_bits();2];ready.send(block).unwrap();
        let mut first=[0.0;6];reader.fill(&mut first,2,|x|x);assert_eq!(reader.shared.beat(),3.03);
        let mut rest=vec![0.0;(BLOCK_FRAMES-3)*2];reader.fill(&mut rest,2,|x|x);
        assert_eq!(reader.shared.beat(),3.0);assert_eq!(returned.len(),1);
        assert_eq!(Shared::get(&reader.shared.meter[0]),0.5);
    }
    #[test]
    fn worker_guard_stops_with_full_prefetch_queue_and_pause_recycles_it() {
        let (tx,rx)=crossbeam_channel::bounded(1024);let shared=Arc::new(Shared::new());
        let worker=WorkerGuard::start(8000.0,rx,shared.clone()).unwrap();
        tx.send(Cmd::Snap(empty_snapshot())).unwrap();tx.send(Cmd::Play(true)).unwrap();
        let wait=|condition:&dyn Fn()->bool| {
            let deadline=std::time::Instant::now()+std::time::Duration::from_secs(3);
            while !condition() {assert!(std::time::Instant::now()<deadline,"worker did not make progress");std::thread::sleep(std::time::Duration::from_millis(1));}
        };
        wait(&||worker.ready.len()==BLOCK_COUNT);assert_eq!(shared.beat(),0.0);
        tx.send(Cmd::Play(false)).unwrap();wait(&||!shared.playing.load(Ordering::Relaxed)&&worker.ready.is_empty());
        tx.send(Cmd::Seek(9.0)).unwrap();tx.send(Cmd::Play(true)).unwrap();wait(&||worker.ready.len()==BLOCK_COUNT);
        assert_eq!(shared.beat(),9.0);drop(worker);
    }
    #[test]
    fn dropping_successful_stream_reader_stops_worker_but_failed_build_does_not() {
        let (_tx,rx)=crossbeam_channel::bounded(1024);let mut worker=WorkerGuard::start(8000.0,rx,Arc::new(Shared::new())).unwrap();
        let failed=worker.reader();assert!(!worker.stop.load(Ordering::Acquire));
        let reader=worker.reader();reader.lifecycle.as_ref().unwrap().0.store(true,Ordering::Release);
        drop(failed);assert!(!worker.stop.load(Ordering::Acquire));drop(reader);
        assert!(worker.stop.load(Ordering::Acquire));
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(3);
        while !worker.thread.as_ref().unwrap().is_finished() {assert!(std::time::Instant::now()<deadline);std::thread::sleep(std::time::Duration::from_millis(1));}
    }
    #[test]
    fn callback_never_waits_for_transport_publication() {
        let (mut reader,control,ready,_returned)=test_reader();reader.shared.playing.store(true,Ordering::Relaxed);
        let mut block=Box::new(AudioBlock::new());block.samples.fill(0.4);ready.send(block).unwrap();
        control.epoch.store(1,Ordering::Release);let mut out=[1.0;2];reader.fill(&mut out,2,|x|x);
        assert_eq!(out,[0.0;2]);assert_eq!(reader.shared.beat(),0.0);assert_eq!(reader.ready.len(),1);assert_eq!(reader.shared.underruns.load(Ordering::Relaxed),0);
        control.epoch.store(0,Ordering::Release);reader.fill(&mut out,2,|x|x);assert_eq!(out,[0.4;2]);
    }
    #[test]
    fn callback_starvation_is_silent_and_does_not_advance_clock() {
        let (mut reader, _owner, ready, _returned) = test_reader();
        reader.shared.beat.store(3.0f64.to_bits(), Ordering::Relaxed);
        reader.shared.playing.store(true, Ordering::Relaxed);
        let mut output=[1.0;14]; reader.fill(&mut output,2,|x|x);
        assert_eq!(output,[0.0;14]); assert_eq!(reader.shared.beat(),3.0);
        assert!(ready.is_empty());assert_eq!(reader.shared.underruns.load(Ordering::Relaxed),1);
        reader.fill(&mut output,2,|x|x);assert_eq!(reader.shared.underruns.load(Ordering::Relaxed),2);
        let mut block=Box::new(AudioBlock::new());block.samples.fill(0.6);ready.send(block).unwrap();
        let mut partial=vec![1.0;(BLOCK_FRAMES+7)*2];reader.fill(&mut partial,2,|x|x);
        assert!(partial[..BLOCK_FRAMES*2].iter().all(|&x|x==0.6));
        assert!(partial[BLOCK_FRAMES*2..].iter().all(|&x|x==0.0));
        assert_eq!(reader.shared.underruns.load(Ordering::Relaxed),3);
    }
    #[test]
    fn epoch_flushes_partial_and_queued_blocks_and_preserves_sample_clock() {
        let (mut reader, control, ready, returned) = test_reader();
        reader.shared.playing.store(true, Ordering::Relaxed);
        let mut old=Box::new(AudioBlock::new()); old.samples.fill(0.7); old.clock=std::array::from_fn(|i|10.0+i as f64); old.end_beat=522.0;
        ready.send(old).unwrap();
        let mut output=[0.0;6]; reader.fill(&mut output,2,|x|x);
        assert_eq!(output,[0.7;6]); assert_eq!(reader.shared.beat(),13.0);
        let mut queued=Box::new(AudioBlock::new()); queued.samples.fill(0.9); ready.send(queued).unwrap();
        control.change(&reader.shared,Some(40.0),None);
        let mut fresh=Box::new(AudioBlock::new()); fresh.epoch=2; fresh.samples.fill(0.3); fresh.clock=std::array::from_fn(|i|40.0+i as f64); fresh.end_beat=552.0; ready.send(fresh).unwrap();
        reader.fill(&mut output,2,|x|x);
        assert_eq!(output,[0.3;6]); assert_eq!(reader.shared.beat(),43.0);
        assert_eq!(returned.len(),2);
    }
    #[test]
    fn pause_flushes_current_output_and_stops_clock() {
        let (mut reader,control,ready,_returned)=test_reader();
        reader.shared.playing.store(true,Ordering::Relaxed);
        let mut block=Box::new(AudioBlock::new());block.samples.fill(0.8);block.clock.fill(2.0);ready.send(block).unwrap();
        let mut out=[0.0;2];reader.fill(&mut out,2,|x|x);
        reader.shared.meter[0].store(0.9f32.to_bits(),Ordering::Relaxed);reader.shared.deck_meter[2].store(0.5f32.to_bits(),Ordering::Relaxed);
        control.change(&reader.shared,None,Some(false));out.fill(1.0);reader.fill(&mut out,2,|x|x);
        assert_eq!(Shared::get(&reader.shared.meter[0]),0.0);assert_eq!(Shared::get(&reader.shared.deck_meter[2]),0.0);
        assert_eq!(out,[0.0;2]);assert_eq!(reader.shared.beat(),2.0);assert_eq!(reader.shared.underruns.load(Ordering::Relaxed),0);
    }
}
