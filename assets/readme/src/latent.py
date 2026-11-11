import math
from common import *
W,H=1200,470; T=18.0
def pc(t): return '%.2f%%'%(t/T*100)
def blob(x,y,n,seed,sd=9):
    r=rng(seed); d=[]
    for i in range(n):
        u=max(1e-6,r()); v=r(); g=math.sqrt(-2*math.log(u))
        px=x+g*math.cos(2*math.pi*v)*sd; py=y+g*math.sin(2*math.pi*v)*sd*.8
        d.append('M%d %dh2v2h-2z'%(round(px),round(py)))
    return ''.join(d)
def fade(name,t_in,t_out,a=1,dur=.6):
    return '@keyframes %s{0%%,%s{opacity:0}%s,%s{opacity:%s}%s,100%%{opacity:0}}'%(name,pc(t_in),pc(t_in+dur),pc(t_out),a,pc(t_out+dur))
def build(mode):
    c=C(mode); dark=mode=='dark'
    css=[]; out=['<rect width="%d" height="%d" fill="%s"/>'%(W,H,c['bg'])]
    out.append(fog(W,H,21,1500,c['fog'],.08,avoid=lambda x,y:120<x<1080 and 40<y<400))
    A=(300,190); B=(860,120); Cn=(900,300); D=(660,330); E=(1060,230)
    # older notes, there from the start
    for (x,y),s,lab in [(B,3,'an older note'),(Cn,5,'an older note'),(D,7,'an older note'),(E,8,'')]:
        out.append('<path fill="%s" fill-opacity="%s" d="%s"/>'%(c['dot'],.85,blob(x,y,34,s)))
        if lab: out.append('<text class="elite meta" x="%d" y="%d" text-anchor="middle">%s</text>'%(x,y+36,lab))
    # the new note, written
    css.append(fade('na',.8,T-1.4,1,1.0))
    out.append('<g class="an" style="animation-name:na"><path fill="%s" d="%s"/><text class="elite meta" x="%d" y="%d" text-anchor="middle">you write a note</text></g>'%(c['dot-hot'],blob(A[0],A[1],46,1,10),A[0],A[1]+40))
    # two suggestions: one shown, one held out and never shown
    sh=bez(A,(A[0]+180,A[1]+90),(Cn[0]-230,Cn[1]+20),(Cn[0]-22,Cn[1]),60)
    ho=bez(A,(A[0]+170,A[1]-80),(B[0]-220,B[1]-30),(B[0]-22,B[1]),60)
    def poly(p): return 'M'+'L'.join('%.1f %.1f'%q for q in p)
    css.append(fade('sh',3.0,T-1.4,1))
    css.append(fade('ho',3.6,T-1.4,1))
    out.append('<g class="an" style="animation-name:sh"><path d="%s" fill="none" stroke="%s" stroke-opacity=".75" stroke-width="1.6" stroke-dasharray="1 7" stroke-linecap="square"/><text class="elite meta" x="%d" y="%d" text-anchor="middle">suggested · shown</text></g>'%(poly(sh),c['thread'],sh[24][0],sh[24][1]+30))
    out.append('<g class="an" style="animation-name:ho"><path d="%s" fill="none" stroke="%s" stroke-opacity=".6" stroke-width="1.4" stroke-dasharray="6 7"/><text class="elite meta" x="%d" y="%d" text-anchor="middle">predicted · held out, never shown</text></g>'%(poly(ho),c['ink-soft'],ho[30][0],ho[30][1]-30))
    # later: the hand draws its own link, and it lands on the held-out prediction
    css.append(fade('lt',6.2,T-1.4,1))
    out.append('<text class="aph an" style="animation-name:lt" x="%d" y="%d" font-size="24" fill="%s">later, by hand</text>'%(150,330,c['ink']))
    hand=bez((A[0]+20,A[1]-10),(A[0]+190,A[1]-95),(B[0]-210,B[1]-40),(B[0]-20,B[1]+2),70)
    L=bezlen(hand)
    bd=brush(hand,3.6,17,.3,.1)
    css.append('@keyframes dr{0%%,%s{stroke-dashoffset:%d}%s,100%%{stroke-dashoffset:0}}'%(pc(7.0),int(L)+10,pc(9.0)))
    css.append('.mk{stroke-dasharray:%d %d;stroke-dashoffset:%d;animation:dr %ds cubic-bezier(.45,0,.3,1) infinite}'%(int(L)+10,int(L)+10,int(L)+10,T))
    out.append('<mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="%d" height="%d"><path class="mk" d="%s" fill="none" stroke="#fff" stroke-width="16" stroke-linecap="round"/></mask>'%(W,H,poly(hand)))
    css.append(fade('hd',6.9,T-1.4,1,.1))
    out.append('<g class="an" style="animation-name:hd"><path mask="url(#m)" fill="%s" d="%s"/></g>'%(c['ink'],bd))
    # the landing: a ring of rubedo, and the word for it
    ring=brush(arc(B[0],B[1],22,-100,250,44),2.2,5,.3)
    css.append('@keyframes rg{0%%,%s{opacity:0;transform:scale(.6)}%s{opacity:1;transform:scale(1)}%s{opacity:1;transform:scale(1)}%s,100%%{opacity:0;transform:scale(1)}}'%(pc(9.0),pc(9.6),pc(T-1.4),pc(T-.8)))
    out.append('<path class="rg" style="transform-origin:%dpx %dpx" fill="%s" d="%s"/>'%(B[0],B[1],c['accent'],ring))
    css.append('.rg{transform-box:view-box;opacity:0;animation:rg %ds linear infinite}'%T)
    css.append(fade('la',9.6,T-1.4,1))
    out.append('<g class="an" style="animation-name:la"><text class="aph" x="%d" y="%d" font-size="26" fill="%s">latent</text><text class="elite meta" x="%d" y="%d">it lands on a prediction it never saw</text></g>'%(B[0]+36,B[1]+8,c['accent-text'],150,358))
    q='what share of the links i later make by hand was already latent in what i had written?'
    out.append('<text class="aph" x="%d" y="%d" text-anchor="middle" font-size="27" fill="%s">%s</text>'%(W/2,430,c['ink'],q))
    css.append('.an{opacity:0;animation-duration:%ds;animation-timing-function:linear;animation-iteration-count:infinite}'%T)
    texts='you write a note an older note suggested · shown predicted · held out, never shown it lands on a prediction it never saw'
    style=face('Aphesis Script',aph(q+'later, by hand latent'))+face('Special Elite',elite(texts))
    style+=".aph{font-family:'Aphesis Script',serif}.elite{font-family:'Special Elite',monospace}.meta{font-size:13px;letter-spacing:1.6px;fill:%s}"%c['ink-soft']
    style+=''.join(css)
    style+='@media (prefers-reduced-motion: reduce){.an,.rg,.mk{animation:none!important;opacity:1!important}.mk{stroke-dashoffset:0!important}.rg{transform:none}}'
    return svg(W,H,'a held-out prediction, and the link made later by hand that lands on it',''.join(out),style)
for m in ('dark','light'): write('latent',m,build(m))
