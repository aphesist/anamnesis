import math, random
from common import *
W,H=1200,560; CX,CY=600,262; SC=470
T=60  # seconds per turn
K=300 # angular sectors
N=2300
random.seed(7)
P=DATA['pts']; regs=DATA['regions']
pts=random.sample(P,N)
LABELS=[1,2,4,7,11,5,9]  # region indices: god·soul, ice·snow, moon·night, slavery·government, poetry·poet, hear·sparrow, emerson·seward
def lab(i):
    w=regs[i]['label'].split(' · ')[:2]; return ' · '.join(w)
def frames(fn,steps=36):
    return ''.join(fn(i/steps) for i in range(steps+1))
def build(mode):
    c=C(mode); dark=mode=='dark'
    sectors=[[[],[]] for _ in range(K)]
    rr=random.Random(3)
    for p in pts:
        x,y,z=p['a']; r=math.hypot(x,z); a=math.atan2(z,x)%(2*math.pi)
        k=int(a/(2*math.pi)*K)%K
        hot=1 if rr.random()<.09 else 0
        sectors[k][hot].append((CX+r*SC, CY-y*SC*1.05))
    css=[]
    lo=.22 if dark else .2
    def kf(u):
        a=2*math.pi*u; cs=math.cos(a); dp=math.sin(a)
        if abs(cs)<.03: cs=.03 if cs>=0 else -.03
        op=lo+(1-lo)*((dp+1)/2)**1.6
        return '%.2f%%{transform:scaleX(%.3f);opacity:%.2f}'%(u*100,cs,op)
    css.append('@keyframes t{'+frames(kf,48)+'}')
    css.append('.s{transform-box:view-box;transform-origin:%dpx %dpx;animation:t %ds linear infinite;fill:none;stroke-linecap:square;vector-effect:non-scaling-stroke}'%(CX,CY,T))
    css.append('.n{stroke:%s;stroke-width:2.1}.h{stroke:%s;stroke-width:2.4}'%(c['dot'],c['dot-hot'] if dark else c['dot-hot']))
    body=[]
    for k in range(K):
        ph=(k+.5)/K  # sector angle as a fraction of a turn
        delay=-ph*T
        for hot in (0,1):
            if not sectors[k][hot]: continue
            d=''.join('M%s %sh0'%(fmt(x),fmt(y)) for x,y in sectors[k][hot])
            a=2*math.pi*ph; cs=math.cos(a); cs=cs if abs(cs)>.03 else math.copysign(.03,cs)
            body.append('<path class="s %s" style="animation-delay:%.1fs;--c:%.3f;--o:%.2f" d="%s"/>'%('h' if hot else 'n',delay,cs,lo+(1-lo)*((math.sin(a)+1)/2)**1.6,d))
    # region labels: centroid of each region, turning with it, legible only while it faces us
    labs=[]
    for j,i in enumerate(LABELS):
        mem=[p for p in P if p['r']==i]
        x=sum(p['a'][0] for p in mem)/len(mem); y=sum(p['a'][1] for p in mem)/len(mem); z=sum(p['a'][2] for p in mem)/len(mem)
        r=math.hypot(x,z); a0=math.atan2(z,x)%(2*math.pi)
        def lkf(u,r=r,a0=a0,j=j):
            a=2*math.pi*u+a0; dp=math.sin(a)
            win=.5+.5*math.cos(2*math.pi*(2*u - j/len(LABELS)))  # each label has its own moments, twice a turn
            op=max(0,min(1,(dp-.35)*2.4))*max(0,min(1,(win-.35)*2.5))
            return '%.2f%%{transform:translateX(%.1fpx);opacity:%.2f}'%(u*100,r*SC*math.cos(a),op)
        css.append('@keyframes l%d{%s}'%(j,frames(lkf,48)))
        css.append('.l%d{animation:l%d %ds linear infinite;opacity:0}'%(j,j,T))
        a=a0; dp=math.sin(a); so=1 if dp>.55 else 0
        labs.append('<text class="aph l%d" style="--x:%.1fpx;--o:%d" x="%d" y="%.1f" text-anchor="middle" font-size="23" fill="%s">%s</text>'%(j,r*SC*math.cos(a),so,CX,CY-y*SC*1.05,c['region-label'],lab(i)))
    word=''
    caption='9,233 dated entries · 34 regions · the transcendentalist demo brain, turning'
    ftext=word+''.join(lab(i) for i in LABELS)
    style=face('Aphesis Script',aph(ftext))+face('Special Elite',elite(caption))
    style+=".aph{font-family:'Aphesis Script',serif}.elite{font-family:'Special Elite',monospace}"
    halo=c['bg']
    style+='.l0,.l1,.l2,.l3,.l4,.l5,.l6{paint-order:stroke;stroke:%s;stroke-width:5px;stroke-linejoin:round;stroke-opacity:.85}'%halo
    style+=''.join(css)
    style+='@media (prefers-reduced-motion: reduce){.s,.aph{animation:none!important}.s{transform:scaleX(var(--c));opacity:var(--o)}.aph[style]{transform:translateX(var(--x));opacity:var(--o)}}'
    out=[]
    out.append(fog(W,H,11,2600,c['fog'],.10 if dark else .09,avoid=lambda x,y:(x-CX)**2/330**2+(y-CY)**2/250**2<1 or (abs(x-CX)<380 and y>505)))
    out+=body+labs
    out.append('<text class="elite" x="%d" y="534" text-anchor="middle" font-size="13" letter-spacing="2.2" fill="%s">%s</text>'%(CX,c['ink-soft'],caption))
    return svg(W,H,'anamnesis: the demo brain turning, made of its own 9,233 entries',''.join(out),style)
for m in ('dark','light'): write('hero',m,build(m))
