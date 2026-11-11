# Builds the animated pictures of the README (assets/readme/*-dark.svg, *-light.svg).
# Run from the repository root, with a built demo brain (see docs/demo.md):
#   uv run --with fonttools --with brotli python assets/readme/src/extract.py <brain data dir>
#   for s in hero growth latent harness; do uv run --with fonttools --with brotli python assets/readme/src/$s.py; done
# extract.py reads views.json, mind_state.json and a copy of brain.sqlite from the data dir into a work file
# ($README_WORK, default: <tmp>/anamnesis-readme/data.json); the other scripts read only that file, the theme
# tokens (ui/theme/dist/aphesis.json), the fonts and the φ.
import math, json, random, base64
from fonts import aph, elite, face
import os, tempfile
ROOT=os.path.abspath(os.path.join(os.path.dirname(__file__),'..','..','..'))+'/'
WORK=os.environ.get('README_WORK',os.path.join(tempfile.gettempdir(),'anamnesis-readme'))+'/'
OUT=ROOT+'assets/readme/'
_t=json.load(open(ROOT+'ui/theme/dist/aphesis.json'))
def C(mode):
    return {k[2:]:v['hex'] for k,v in _t['modes'][mode].items() if v.get('type')=='color'}
DATA=json.load(open(WORK+'data.json'))
def rng(seed):
    s=[(seed & 0xffffffff) or 1]
    def r():
        x=s[0]; x^=(x<<13)&0xffffffff; x^=x>>17; x^=(x<<5)&0xffffffff; s[0]=x; return x/4294967296
    return r
def brush(pts,w,seed=1,taper=.22,amp=.08):
    r=rng(seed); n=len(pts); L=[];R=[]; ph=[r()*6,r()*6,r()*6]
    for i in range(n):
        a=pts[max(0,i-1)]; b=pts[min(n-1,i+1)]; tx=b[0]-a[0]; ty=b[1]-a[1]; l=math.hypot(tx,ty) or 1; tx/=l; ty/=l
        u=i/(n-1); k=1
        if taper>0 and u<taper: k=math.sin(u/taper*math.pi/2)
        if taper>0 and u>1-taper: k=min(k,math.sin((1-u)/taper*math.pi/2))
        wob=1+amp*(math.sin(u*9+ph[0])+.5*math.sin(u*23+ph[1]))
        wl=w*k*wob*(1+.06*math.sin(u*31+ph[2])); wr=w*k*wob*(1-.05*math.sin(u*27+ph[1]))
        L.append((pts[i][0]-ty*wl/2,pts[i][1]+tx*wl/2)); R.append((pts[i][0]+ty*wr/2,pts[i][1]-tx*wr/2))
    f=lambda v:('%.1f'%v).rstrip('0').rstrip('.')
    d='M'+f(L[0][0])+' '+f(L[0][1])+''.join('L'+f(x)+' '+f(y) for x,y in L[1:])
    d+=''.join('L'+f(x)+' '+f(y) for x,y in reversed(R))
    return d+'Z'
def line(x0,y0,x1,y1,n=24): return [(x0+(x1-x0)*i/n,y0+(y1-y0)*i/n) for i in range(n+1)]
def bez(p0,p1,p2,p3,n=40):
    o=[]
    for i in range(n+1):
        t=i/n; m=1-t
        o.append((m**3*p0[0]+3*m*m*t*p1[0]+3*m*t*t*p2[0]+t**3*p3[0], m**3*p0[1]+3*m*m*t*p1[1]+3*m*t*t*p2[1]+t**3*p3[1]))
    return o
def bezlen(p):
    return sum(math.hypot(p[i+1][0]-p[i][0],p[i+1][1]-p[i][1]) for i in range(len(p)-1))
def fmt(v): 
    s='%.1f'%v
    return s[:-2] if s.endswith('.0') else s
PHI_D=open(ROOT+'ui/theme/themes/aphesis/assets/phi.svg').read().split(' d="')[1].split('"')[0]
def phi(x,y,h,color):
    # the φ mark, viewBox 69 -6 385 533, scaled to height h with its top-left at (x,y)
    k=h/533
    return '<path fill="%s" fill-rule="evenodd" transform="translate(%s %s) scale(%.5f) translate(-69 6)" d="%s"/>'%(color,fmt(x),fmt(y),k,PHI_D)
def fog(w,h,seed,count,color,op,avoid=None,size=2):
    # sparse stipple ground: square cells, denser toward the edges
    r=rng(seed); d=[]
    for i in range(count):
        x=r()*w; y=r()*h
        e=min(x,w-x,y*2.2,(h-y)*2.2)/(w/2)
        if r()<e*1.6: continue
        if avoid and avoid(x,y): continue
        x=int(x/3)*3; y=int(y/3)*3
        d.append('M%d %dh%dv%dh-%dz'%(x,y,size,size,size))
    return '<path fill="%s" fill-opacity="%s" d="%s"/>'%(color,op,''.join(d))
def svg(w,h,title,body,style):
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %d %d" width="%d" height="%d" role="img"><title>%s</title><style>%s</style>%s</svg>'%(w,h,w,h,title,style,body))
RM='@media (prefers-reduced-motion: reduce){*{animation:none!important}}'
def write(name,mode,s):
    import os; os.makedirs(OUT,exist_ok=True)
    p=OUT+'%s-%s.svg'%(name,mode); open(p,'w').write(s); print(p,len(s))
def project(p, yaw=0.6, pitch=0.14, dist=4.5, f=1.0):
    cp=math.cos(pitch); ey=[dist*cp*math.sin(yaw),dist*math.sin(pitch),dist*cp*math.cos(yaw)]
    fw=[-e/dist for e in ey]; up=[0,1,0]
    r=[fw[1]*up[2]-fw[2]*up[1],fw[2]*up[0]-fw[0]*up[2],fw[0]*up[1]-fw[1]*up[0]]; n=math.hypot(*r); r=[v/n for v in r]
    u=[r[1]*fw[2]-r[2]*fw[1],r[2]*fw[0]-r[0]*fw[2],r[0]*fw[1]-r[1]*fw[0]]
    q=[p[k]-ey[k] for k in range(3)]
    zc=sum(q[k]*fw[k] for k in range(3))
    return (sum(q[k]*r[k] for k in range(3))/zc*f, sum(q[k]*u[k] for k in range(3))/zc*f, zc)
def arc(cx,cy,r,a0,a1,n=48,ry=None):
    return [(cx+r*math.cos(math.radians(a0+(a1-a0)*i/n)), cy+(ry or r)*math.sin(math.radians(a0+(a1-a0)*i/n))) for i in range(n+1)]
