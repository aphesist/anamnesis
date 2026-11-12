import math, random
from common import *
W,H=1200,600; CX,CY=600,205
T=30.0; Y0,Y1=1820,1877; RUN=22.0  # seconds of the loop spent growing
random.seed(11)
P=DATA['pts']; regs=DATA['regions']
pts=sorted(random.sample(P,2600),key=lambda p:p['t'])
BIN=0.5
def tt(year): return (year-Y0)/(Y1-Y0)*RUN
def pc(t): return '%.2f%%'%(t/T*100)
TL0,TL1,TLY=150,1050,528
def tx(year): return TL0+(year-Y0)/(Y1-Y0)*(TL1-TL0)
LAB=[(1,'god · soul'),(2,'ice · snow'),(4,'moon · night'),(7,'slavery · government'),(11,'poetry · poet'),(10,'pines · wood'),(14,'england · english'),(23,'merrimack · river'),(18,'nest · eggs'),(9,'emerson · seward')]
def build(mode):
    c=C(mode); dark=mode=='dark'
    css=[]; out=['<rect width="%d" height="%d" fill="%s"/>'%(W,H,c['bg'])]
    out.append(fog(W,H,5,1800,c['fog'],.08 if dark else .08,avoid=lambda x,y:(x-CX)**2/360**2+(y-CY)**2/230**2<1 or y>380))
    # dots, binned by half-year of writing
    bins={}
    for p in pts:
        b=int((p['t']-Y0)/BIN); x,y,_=project(p['a'],f=1600)
        bins.setdefault(b,[]).append((CX+x,CY-y))
    defs=[]; dots=[]; flashes=[]
    for b,ps in sorted(bins.items()):
        t0=tt(Y0+b*BIN)
        d=''.join('M%s %sh0'%(fmt(x),fmt(y)) for x,y in ps)
        defs.append('<path id="b%d" d="%s"/>'%(b,d))
        css.append('@keyframes a%d{0%%,%s{opacity:0}%s,%s{opacity:1}%s{opacity:0}}'%(b,pc(t0),pc(t0+.25),pc(T-2.2),pc(T-.8)))
        css.append('@keyframes f%d{0%%,%s{opacity:0}%s{opacity:1}%s,100%%{opacity:0}}'%(b,pc(t0),pc(t0+.1),pc(t0+.9)))
        dots.append('<use href="#b%d" class="d" style="animation-name:a%d"/>'%(b,b))
        flashes.append('<use href="#b%d" class="fl" style="animation-name:f%d"/>'%(b,b))
    css.append('.d,.fl{animation-duration:%ds;animation-timing-function:linear;animation-iteration-count:infinite}'%T)
    css.append('.d{stroke:%s;stroke-width:2.2;stroke-linecap:square;fill:none}'%c['dot'])
    css.append('.fl{stroke:%s;stroke-width:3.4;stroke-linecap:square;fill:none;opacity:0}'%c['dot-hot'])
    # region births: the date each region gathers its fifth entry, from the real dates
    rings=[]
    for i,r in enumerate(regs):
        mem=sorted([p for p in P if p['r']==i],key=lambda p:p['t'])
        if len(mem)<5: continue
        born=mem[4]['t']
        cx=sum(p['a'][0] for p in mem)/len(mem); cy=sum(p['a'][1] for p in mem)/len(mem); cz=sum(p['a'][2] for p in mem)/len(mem)
        x,y,_=project((cx,cy,cz),f=1600); x+=CX; y=CY-y
        t0=tt(born)
        css.append('@keyframes r%d{0%%,%s{opacity:0;transform:scale(.5)}%s{opacity:1;transform:scale(1)}%s,100%%{opacity:0;transform:scale(1.5)}}'%(i,pc(t0),pc(t0+.4),pc(t0+2.0)))
        rings.append('<path class="ring" style="animation-name:r%d;transform-origin:%.0fpx %.0fpx" d="%s"/>'%(i,x,y,brush(arc(x,y,13,-80,265,40),2.0,100+i,.3)))
        lab=[l for j,l in LAB if j==i]
        if lab:
            css.append('@keyframes n%d{0%%,%s{opacity:0}%s,%s{opacity:1}%s,100%%{opacity:0}}'%(i,pc(t0),pc(t0+.5),pc(t0+3.2),pc(t0+4.4)))
            rings.append('<text class="aph rl" style="animation-name:n%d" x="%.0f" y="%.0f" text-anchor="middle">%s</text>'%(i,x,y-22,'birth · '+lab[0]))
        # a tick on the timeline where the region was born
        rings.append('<path fill="%s" d="%s"/>'%(c['ink-soft'],brush(line(tx(born),TLY+3,tx(born),TLY+8,5),1.2,300+i,.3)))
    css.append('.ring,.rl{animation-duration:%ds;animation-timing-function:linear;animation-iteration-count:infinite;opacity:0}'%T)
    css.append('.ring{fill:%s;transform-box:view-box}'%c['accent'])
    css.append('.rl{font-size:20px;fill:%s;paint-order:stroke;stroke:%s;stroke-width:5px;stroke-linejoin:round}'%(c['region-label'],c['bg']))
    # the timeline: writing per year as stipple columns, a brushed baseline, decade ticks
    years={}
    for p in P: years[int(p['t'])]=years.get(int(p['t']),0)+1
    cells=[]; rr=rng(9)
    for yv,n in years.items():
        h=int(round(math.sqrt(n)*1.05))
        for k in range(h):
            for j in range(3):
                if rr()<.62: cells.append('M%d %dh2v2h-2z'%(int(tx(yv)+j*4+rr()*2),TLY-8-k*4-int(rr()*2)))
    out.append('<path fill="%s" fill-opacity="%s" d="%s"/>'%(c['fog'],'.55' if dark else '.6',''.join(cells)))
    out.append('<path fill="%s" d="%s"/>'%(c['ink-soft'],brush(line(TL0-10,TLY,TL1+10,TLY,60),1.6,77,.05,.04)))
    for yv in range(1820,1880,10):
        out.append('<path fill="%s" d="%s"/>'%(c['ink-soft'],brush(line(tx(yv),TLY+2,tx(yv),TLY+9,6),1.3,yv,.2)))
        out.append('<text class="elite" x="%.0f" y="%d" text-anchor="middle" font-size="12" letter-spacing="1.5" fill="%s">%d</text>'%(tx(yv),TLY+26,c['ink-soft'],yv))
    out.append('<text class="aph" x="%d" y="%d" font-size="24" fill="%s">when you wrote</text>'%(TL0-10,TLY-62,c['ink']))
    # the cursor: a thin brushed hand that walks the years, then rests
    cur=brush(line(TL0,TLY-50,TL0,TLY+6,16),1.8,55,.25)
    css.append('@keyframes cu{0%%{transform:translateX(0);opacity:1}%s{transform:translateX(%dpx);opacity:1}%s{transform:translateX(%dpx);opacity:0}100%%{transform:translateX(0);opacity:0}}'%(pc(RUN),TL1-TL0,pc(T-.8),TL1-TL0))
    css.append('.cu{animation:cu %ds linear infinite;fill:%s}'%(T,c['accent']))
    out.append('<path class="cu" d="%s"/>'%cur)
    out.append('<defs>'+''.join(defs)+'</defs>')
    out+=dots+flashes+rings
    cap='1820 to 1876 · each dot appears on the day it was written · a red ring is a region being born'
    out.append('<text class="elite" x="%d" y="%d" text-anchor="middle" font-size="12" letter-spacing="1.8" fill="%s">%s</text>'%(CX,H-12,c['ink-soft'],cap))
    ftext='when you wrote birth · '+''.join(l for _,l in LAB)
    style=face('Aphesis Script',aph(ftext))+face('Special Elite',elite(cap+'0123456789'))
    style+=".aph{font-family:'Aphesis Script',serif}.elite{font-family:'Special Elite',monospace}"
    style+=''.join(css)
    style+='@media (prefers-reduced-motion: reduce){.d,.fl,.ring,.rl,.cu{animation:none!important}.d{opacity:1}.cu{transform:translateX(%dpx)}}'%(TL1-TL0)
    return svg(W,H,'the demo brain growing in the order its entries were written, 1820 to 1876',''.join(out),style)
for m in ('dark','light'): write('growth',m,build(m))
