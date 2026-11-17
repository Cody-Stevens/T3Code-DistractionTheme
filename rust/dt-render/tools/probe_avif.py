"""Generate synthetic input and measure Pillow, without touching the live helper."""
import subprocess,time,io
from pathlib import Path
import numpy as np
from PIL import Image
out=Path(__file__).resolve().parents[2]/'target'/'render-measure'; out.mkdir(exist_ok=True)
raw=subprocess.run(['ffmpeg','-v','error','-f','lavfi','-i','testsrc2=size=640x360:rate=24','-frames:v','8','-f','rawvideo','-pix_fmt','rgb24','-threads','2','-'],capture_output=True,check=True).stdout
(out/'input.rgb').write_bytes(raw)
frames=[Image.frombytes('RGB',(640,360),raw[i*640*360*3:(i+1)*640*360*3]) for i in range(8)]
for speed in (10,6):
 for q in (20,35,50):
  ts=[]
  for _ in range(3):
   b=io.BytesIO();t=time.perf_counter();frames[0].save(b,'AVIF',save_all=True,append_images=frames[1:],duration=42,loop=1,quality=q,speed=speed,max_threads=4);ts.append(1000*(time.perf_counter()-t))
  data=b.getvalue();(out/f'pillow-s{speed}-q{q}.avif').write_bytes(data)
  dec=Image.open(io.BytesIO(data)); a=[]
  for i in range(dec.n_frames):dec.seek(i);a.append(np.asarray(dec.convert('RGB')))
  psnr=10*np.log10(255**2/np.mean((np.stack(a).astype(float)-np.frombuffer(raw,np.uint8).reshape(8,360,640,3))**2))
  print('pillow',speed,q,len(data),round(psnr,2),[round(t,1) for t in ts],flush=True)
