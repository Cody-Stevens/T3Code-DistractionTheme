from pathlib import Path
import sys
root=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(root/'python'))
import theme_css as c
import card_image
p=Path(__file__).resolve().parents[1]
css=c.static_css(800,450)
a=css.index('@keyframes gtjump');b=css.index(c.CONV+' :is(.chat-markdown',a)
(p/'src'/'text_flow.css').write_text(css[a:b],encoding='utf8')
(p/'src'/'static.css').write_text(css[:a].replace('--vw:800px','--vw:{VIDEO_W}px').replace('--vh:450px','--vh:{VIDEO_H}px')+'{TEXT_FLOW}'+css[b:],encoding='utf8')
frames=[{'aa':'#abcdef','b0':'0.25','bs':'0.5'},{},{'aa':'#123456','kk':'1','pr':'0.75'}]
# Sort only to match the public Rust BTreeMap contract.
frames=[dict(sorted(f.items())) for f in frames]
fixtures={'static':css,'static_small':c.static_css(320,180),'idle':c.idle_css(),'keyframes':c.keyframe_block('d42',frames),'animate':c.animate('d42',8/24),'infinite':c.animate('d42',16,'infinite'),'values':c.static_values(frames[0]),'height':c.card_height_css(285),'card':c.card_image_css(card_image.Card('YWJj',285)),'glow':c.glow_css('YWJj'),'front_a':c.favicon_front_css('a'),'front_b':c.favicon_front_css('b')}
# Keep the idle palette in production as the exact fixed Python output.
(p/"src"/"idle.css").write_text(c.idle_css(),encoding="utf8")
for name,data in fixtures.items():(p/'tests'/'golden'/f'{name}.css').write_text(data,encoding='utf8')
