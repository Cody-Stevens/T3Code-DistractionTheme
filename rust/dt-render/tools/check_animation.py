"""Real-time Edge headless screenshot sampling. Own browser profile and whole-tree cleanup."""
import argparse,base64,json,subprocess,time,urllib.request
from pathlib import Path
import websocket
p=argparse.ArgumentParser();p.add_argument('directory',type=Path);args=p.parse_args();directory=args.directory.resolve()
profile=directory/'edge-profile';profile.mkdir(exist_ok=True)
(profile/'DevToolsActivePort').unlink(missing_ok=True)
edge=Path('C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe')
proc=subprocess.Popen([str(edge),'--headless=new','--no-first-run','--disable-background-networking','--disable-extensions','--disable-background-timer-throttling','--disable-renderer-backgrounding','--remote-allow-origins=*','--remote-debugging-port=0','--user-data-dir='+str(profile),'about:blank'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
try:
 deadline=time.monotonic()+15
 portfile=profile/'DevToolsActivePort'
 while not portfile.exists():
  if time.monotonic()>deadline:raise RuntimeError('Edge did not open DevTools')
  time.sleep(.1)
 port=portfile.read_text().splitlines()[0]
 tab=json.load(urllib.request.urlopen(f'http://127.0.0.1:{port}/json/list'))[0]
 ws=websocket.create_connection(tab['webSocketDebuggerUrl'],timeout=30,origin='http://localhost')
 ids=0
 def cdp(method,params):
  global ids
  ids+=1;ws.send(json.dumps({'id':ids,'method':method,'params':params}))
  while True:
   msg=json.loads(ws.recv())
   if msg.get('id')==ids:
    if 'error' in msg:raise RuntimeError(msg['error'])
    return msg.get('result')
 files=[directory/x for x in ['pillow-s10-q35.avif','generic-s10-q35.avif','rust-native-fast-q35-0.avif','rust-persistent-fast-q35-5.avif','rust-persistent-ahead-q35-0.avif','svt-q50.avif','rav1e-ffmpeg-q180.avif','rav1e-rust-noasm-q220.avif','native-exact.avif','favicon.svg'] if (directory/x).exists()]
 # Navigate to the local directory so the test uses file-origin asset access without a server.
 page=directory/'animation.html';page.write_text('<!doctype html><meta charset="utf-8"><body>AVIF animation probe</body>',encoding='utf8')
 cdp('Page.navigate',{'url':page.as_uri()});time.sleep(.25)
 cdp('Page.bringToFront',{})
 cdp('Emulation.setDeviceMetricsOverride',{'width':800,'height':600,'deviceScaleFactor':1,'mobile':False})
 rows=[]
 for file in files:
  src='data:'+('image/svg+xml' if file.suffix=='.svg' else 'image/avif')+';base64,'+base64.b64encode(file.read_bytes()).decode()
  result=cdp('Runtime.evaluate',{'expression':"document.body.innerHTML='';var im=new Image();im.style='position:fixed;left:0;top:0;width:640px;height:360px';document.body.append(im);im.src="+json.dumps(src)})
  shots=[];times=[];begin=time.monotonic()
  for i in range(12):
   data=cdp('Page.captureScreenshot',{'format':'png','captureBeyondViewport':False})['data']
   png=base64.b64decode(data)
   import hashlib
   shots.append(hashlib.sha256(png).hexdigest());times.append(round((time.monotonic()-begin)*1000))
   if i==2:(directory/(file.name+'.png')).write_bytes(png)
   time.sleep(.04)
  row={'file':file.name,'distinct':len(set(shots)),'heldLast':len(set(shots[-4:]))==1,'times':times};rows.append(row);print(row,flush=True)
 version=cdp('Browser.getVersion',{})
 (directory/'chromium.json').write_text(json.dumps({'version':version,'results':rows},indent=2),encoding='utf8')
 ws.close()
finally:
 subprocess.run(['taskkill','/T','/F','/PID',str(proc.pid)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);proc.wait(timeout=10)
