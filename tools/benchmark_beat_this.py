#!/usr/bin/env python3
"""Opt-in offline Beat This! benchmark; raw reports contain local paths.

Install the official beat-this package in an isolated environment first.
No MixStudio project, source file or analysis cache is changed.
"""
import argparse, os
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--manifest',required=True,help='One local audio path per line')
parser.add_argument('--output',required=True,help='Private results directory')
parser.add_argument('--model',default='final0')
parser.add_argument('--threads',type=int,default=4)
parser.add_argument('--synthetic',action='store_true',help='Also evaluate authored breakdown and tempo-ramp references')
args=parser.parse_args()
if args.threads<1:parser.error('--threads must be positive')
os.environ.setdefault('TORCH_HOME',str(__import__('pathlib').Path(args.output)/'model-cache'))
import time,json,pathlib,hashlib,subprocess,numpy as np,torch,soundfile as sf
from beat_this.inference import Audio2Beats
out=pathlib.Path(args.output);out.mkdir(parents=True,exist_ok=True)
torch.set_num_threads(args.threads);torch.set_num_interop_threads(1)
model_start=time.perf_counter();tracker=Audio2Beats(checkpoint_path=args.model,device='cpu',dbn=False)
print('Model loaded',time.perf_counter()-model_start,flush=True)
paths=[p.strip() for p in pathlib.Path(args.manifest).read_text().splitlines() if p.strip() and not p.lstrip().startswith('#')]
references={}
fixtures=pathlib.Path(__import__("tempfile").mkdtemp(prefix="authored-",dir=out)) if args.synthetic else out
for mode in (['steady-breakdown','tempo-ramp'] if args.synthetic else []):
 sr=22050;p=60/126;times=[.3]
 for k in range(255):times.append(times[-1]+(p if mode=='steady-breakdown' else 60/(120+8*k/254)))
 x=np.zeros(int((times[-1]+1)*sr),np.float32)
 for k,t in enumerate(times):
  if mode=='steady-breakdown' and 80<=k<144:continue
  j=np.arange(int(.18*sr));dt=j/sr
  kick=(1 if k%4==0 else .55)*.6*np.sin(2*np.pi*(50+100*np.exp(-30*dt))*dt)*np.exp(-20*dt)
  x[int(t*sr):int(t*sr)+len(j)]+=kick.astype(np.float32)
 if mode=='steady-breakdown':
  for k in range(80,144):
   at=int((times[k]+p*.5)*sr);j=np.arange(int(.04*sr));dt=j/sr;x[at:at+len(j)]+=.35*np.sin(2*np.pi*1700*dt)*np.exp(-dt*145)
 path=str(fixtures/(mode+'.wav'));sf.write(path,x,sr,subtype='FLOAT');references[path]=times;paths.append(path)
results=[]
for path in paths:
 print('Analysing',path,flush=True)
 source_sha256=hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()
 raw=subprocess.check_output(['ffmpeg','-v','error','-i',path,'-f','f32le','-ar','22050','-ac','2','pipe:1'])
 audio=np.frombuffer(raw,dtype='<f4').reshape(-1,2)
 start=time.perf_counter();beats,downbeats=tracker(audio,22050);elapsed=time.perf_counter()-start
 beats=np.asarray(beats);downbeats=np.asarray(downbeats)
 spans=beats[32:]-beats[:-32];bpm=float(60*32/np.median(spans)) if len(spans) else None
 if hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()!=source_sha256:raise RuntimeError('Source changed during analysis: '+path)
 entry={'path':path,'source_sha256':source_sha256,'seconds':len(audio)/22050,'elapsed_seconds':elapsed,'beats':beats.tolist(),'downbeats':downbeats.tolist(),'bpm_32beat_median':bpm}
 if path in references:
  truth=np.array(references[path]);err=np.min(abs(truth[:,None]-beats[None,:]),axis=1) if len(beats) else np.full(len(truth),np.inf)
  dbtruth=truth[::4];dberr=np.min(abs(dbtruth[:,None]-downbeats[None,:]),axis=1) if len(downbeats) else np.full(len(dbtruth),np.inf)
  entry['authored_reference']={'beats':truth.tolist(),'median_error_ms':float(np.median(err)*1000),'p95_error_ms':float(np.quantile(err,.95)*1000),'within25ms':float(np.mean(err<=.025)),'downbeats_within25ms':float(np.mean(dberr<=.025))}
 results.append(entry)
 report={'model':args.model,'package_version':__import__('importlib.metadata',fromlist=['version']).version('beat-this'),'torch':torch.__version__,'torchaudio':__import__('importlib.metadata',fromlist=['version']).version('torchaudio'),'device':'cpu','threads':args.threads,'dbn':False,'files':results,'note':'Real files have no annotated beat/downbeat truth. Authored reference times precede inference.'}
 (out/'results.json').write_text(json.dumps(report,indent=2))
 print('DONE',pathlib.Path(path).name,'elapsed',round(elapsed,2),'beats',len(beats),'downbeats',len(downbeats),'bpm',bpm,'reference',entry.get('authored_reference',{}).get('p95_error_ms'),flush=True)
