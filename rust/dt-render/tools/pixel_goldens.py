import ast,sys,colorsys
from pathlib import Path
import numpy as np
from PIL import Image,ImageFilter
root=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(root/'python'));import theme_css
text=(root/'python'/'glowtube.py').read_text(encoding='utf8');tree=ast.parse(text)
funcs=[x for x in tree.body if isinstance(x,ast.FunctionDef) and x.name in ['light','edge_colors','glow_frame']]
ns=dict(np=np,Image=Image,ImageFilter=ImageFilter,colorsys=colorsys,GLOW_W=128,GLOW_H=72,theme_css=theme_css,GLOW_PX=25)
y,x=np.mgrid[:72,:128].astype(np.float32);ns['_GDIST']=np.hypot(x-63.5,y-35.5);exec(compile(ast.Module(body=funcs,type_ignores=[]),'reference','exec'),ns)
f=np.fromfunction(lambda y,x,c:(x*7+y*13+c*67)%256,(36,64,3)).astype(np.uint8)
print('edge',ns['edge_colors'](f))
for features in [(0,0,0),(.7,.8,280),(1,1,750)]:
 out=np.asarray(ns['glow_frame'](f,*features));pts=[out[y,x].tolist() for y,x in [(0,0),(10,20),(35,40),(35,63),(40,90),(60,100),(71,127)]];print(features,pts)
