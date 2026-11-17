from pathlib import Path
from PIL import Image
import numpy as np
p=Path(__file__).resolve().parents[2]/'target'/'render-measure';ref=np.frombuffer((p/'input.rgb').read_bytes(),np.uint8).reshape(8,360,640,3)
rows={}
for label,pattern in [('Pillow10','pillow-s10-q{}.avif'),('Pillow6','pillow-s6-q{}.avif'),('Native10','rust-native-fast-q{}-0.avif'),('Native6','rust-native-ahead-q{}-0.avif'),('Persistent','rust-persistent-fast-q{}-5.avif'),('OneShot','rust-oneshot-fast-q{}-0.avif'),('FFmpegAhead','rust-persistent-ahead-q{}-0.avif')]:
 pts=[]
 for q in [20,35,50]:
  f=p/pattern.format(q)
  if not f.exists():break
  im=Image.open(f);out=[]
  for i in range(im.n_frames):im.seek(i);out.append(np.asarray(im.convert('RGB')))
  psnr=10*np.log10(255**2/np.mean((np.stack(out).astype(float)-ref)**2));pts.append((f.stat().st_size,float(psnr)))
 if not pts:continue
 rows[label]=sorted(pts);print(label,[(n,round(q,3)) for n,q in pts])
for label,pattern,qualities in [('SVT','svt-q{}.avif',[30,40,50,60]),('Rav1eFFmpeg','rav1e-ffmpeg-q{}.avif',[80,130,180]),('Rav1eNoAsm','rav1e-rust-noasm-q{}.avif',[80,130,180,220,240])]:
 pts=[]
 for q in qualities:
  f=p/pattern.format(q)
  if not f.exists():continue
  im=Image.open(f);out=[]
  for i in range(im.n_frames):im.seek(i);out.append(np.asarray(im.convert('RGB')))
  score=10*np.log10(255**2/np.mean((np.stack(out).astype(float)-ref)**2));pts.append((f.stat().st_size,float(score)))
 if pts:rows[label]=sorted(pts);print(label,[(n,round(q,3)) for n,q in pts])
for n in [12000,18000,24000]:
 print('matched',n,{label:round(float(np.interp(n,[s for s,_ in pts],[q for _,q in pts])),3) for label,pts in rows.items() if pts[0][0]<=n<=pts[-1][0]})

quality=28.036
print('bytes at',quality,'dB',{label:round(float(np.interp(quality,[q for _,q in sorted(pts,key=lambda p:p[1])],[n for n,_ in sorted(pts,key=lambda p:p[1])]))) for label,pts in rows.items() if min(q for _,q in pts)<=quality<=max(q for _,q in pts)})
