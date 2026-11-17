"""Compare installed ffmpeg encoders with Pillow on generated eight-frame media. No live state."""
from pathlib import Path
import subprocess,time,io,json,statistics
import numpy as np
from PIL import Image
p=Path(__file__).resolve().parents[2]/'target'/'render-measure';raw=(p/'input.rgb').read_bytes();ref=np.frombuffer(raw,np.uint8).reshape(8,360,640,3)
def score(file):
 im=Image.open(file);a=[]
 for i in range(im.n_frames):im.seek(i);a.append(np.asarray(im.convert('RGB')))
 return float(10*np.log10(255**2/np.mean((np.stack(a).astype(float)-ref)**2)))
rows=[]
for label,encoder,extra,qs in [('svt','libsvtav1',['-preset','12','-svtav1-params','pred-struct=1:lp=4'],[30,40,50,60]),('rav1e-ffmpeg','librav1e',['-speed','10','-threads','4'],[80,130,180])]:
 for q in qs:
  file=p/f'{label}-q{q}.avif';args=['ffmpeg','-v','error','-f','rawvideo','-pix_fmt','rgb24','-s','640x360','-r','1000/42','-i','pipe:0','-pix_fmt','yuv420p','-c:v',encoder]+extra+(['-qp',str(q)] if encoder=='librav1e' else ['-crf',str(q)])+['-f','avif','-loop','1','-y',str(file)]
  ts=[]
  for _ in range(5):
   t=time.perf_counter();result=subprocess.run(args,input=raw,capture_output=True)
   if result.returncode:print(label,result.stderr.decode(errors='replace')[-1000:]);break
   ts.append((time.perf_counter()-t)*1000)
  if ts:
   row={'encoder':label,'q':q,'bytes':file.stat().st_size,'psnr':score(file),'median_ms':statistics.median(ts),'worst_ms':max(ts)};rows.append(row);print(row,flush=True)
(p/'alternatives.json').write_text(json.dumps(rows,indent=2))
