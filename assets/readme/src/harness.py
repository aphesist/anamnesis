import math, random
from common import *
W,H=1200,500; BX,BY=930,235
ROUTES=[('local models','ollama · llama.cpp · lm studio · vllm · the default',0),
        ('api keys','anthropic · openai-compatible · openrouter · gemini',0),
        ('continue with chatgpt',"openai's sign-in for local apps · chat only",0),
        ('your installed claude cli','opt-in · chat only',1),
        ('the brain as an mcp server','claude code · codex · gemini cli · claude desktop',2)]
def build(mode):
    c=C(mode); dark=mode=='dark'
    css=[]; out=['<rect width="%d" height="%d" fill="%s"/>'%(W,H,c['bg'])]
    out.append(fog(W,H,31,1500,c['fog'],.08,avoid=lambda x,y:60<x<1140 and 30<y<460))
    # the brain: a still sample of the real demo brain, breathing
    random.seed(5); pts=random.sample(DATA['pts'],900)
    d=[]; d2=[]
    for i,p in enumerate(pts):
        x,y,z=project(p['a'],yaw=0.6,f=1050)
        (d2 if z<4.45 else d).append('M%s %sh0'%(fmt(BX+x),fmt(BY-y)))
    out.append('<path class="br" stroke="%s" stroke-width="2" stroke-linecap="square" fill="none" stroke-opacity=".55" d="%s"/>'%(c['dot'],''.join(d)))
    out.append('<path class="br b2" stroke="%s" stroke-width="2.2" stroke-linecap="square" fill="none" d="%s"/>'%(c['dot'],''.join(d2)))
    css.append('@keyframes br{0%,100%{opacity:.75}50%{opacity:1}}.b2{animation:br 7s ease-in-out infinite}')
    out.append('<text class="aph" x="%d" y="%d" text-anchor="middle" font-size="24" fill="%s">your brain</text>'%(BX,BY+165,c['ink']))
    out.append('<text class="elite meta" x="%d" y="%d" text-anchor="middle">one sqlite file, on your machine</text>'%(BX,BY+188))
    # the routes and their threads
    y0=78; dy=78
    for i,(name,detail,kind) in enumerate(ROUTES):
        y=y0+i*dy
        grey=kind==1
        out.append('<text class="aph" x="60" y="%d" font-size="25" fill="%s">%s</text>'%(y,c['ink'],name))
        out.append('<text class="elite meta" x="62" y="%d">%s</text>'%(y+22,detail))
        if grey:
            out.append('<text class="aph" x="%d" y="%d" font-size="21" fill="%s">grey</text>'%(60+len(name)*8.5+14,y,c['ink-soft']))
        x0=60+len(name)*11.4+(64 if grey else 18)
        x0=max(x0,470)
        ex=BX-150+abs(i-2)*14; ey=BY+(i-2)*34
        p=bez((x0,y-8),(x0+120,y-8),(ex-110,ey),(ex,ey),50)
        dd='M'+'L'.join('%.1f %.1f'%q for q in p)
        col=c['ink-soft'] if grey else c['thread']
        op='.45' if grey else '.9'
        out.append('<path d="%s" fill="none" stroke="%s" stroke-opacity=".18" stroke-width="1"/>'%(dd,col))
        cls='fw' if kind!=2 else 'fw'
        out.append('<path class="th %s" style="animation-delay:-%.1fs" d="%s" fill="none" stroke="%s" stroke-opacity="%s" stroke-width="2.2" stroke-linecap="square" stroke-dasharray="0.1 %s"/>'%(cls,i*0.7,dd,col,op,'14' if not grey else '22'))
        if kind==2:
            # the mcp server answers back: a second thread flowing out to the client
            out.append('<path class="th bw" d="%s" fill="none" stroke="%s" stroke-opacity=".7" stroke-width="2.2" stroke-linecap="square" stroke-dasharray="0.1 19"/>'%(dd,c['dot']))
    css.append('@keyframes fw{to{stroke-dashoffset:-%d}}@keyframes bw{to{stroke-dashoffset:%d}}'%(14*4*3,19*3*2))
    css.append('.fw{animation:fw 6s linear infinite}.bw{animation:bw 7s linear infinite}')
    cap='local first · every remote call is logged as an egress event: counts, never text'
    out.append('<text class="elite meta" x="%d" y="%d" text-anchor="middle">%s</text>'%(W/2,H-22,cap))
    texts=''.join(d for _,d,_ in ROUTES)+cap+'one sqlite file, on your machine'
    style=face('Aphesis Script',aph(''.join(n for n,_,_ in ROUTES)+'grey your brain'))+face('Special Elite',elite(texts))
    style+=".aph{font-family:'Aphesis Script',serif}.elite{font-family:'Special Elite',monospace}.meta{font-size:14px;letter-spacing:1.3px;fill:%s}"%c['ink-soft']
    style+=''.join(css)
    style+='@media (prefers-reduced-motion: reduce){.th,.b2{animation:none!important}}'
    return svg(W,H,'the routes to a model, and the brain as an mcp server',''.join(out),style)
for m in ('dark','light'): write('harness',m,build(m))
