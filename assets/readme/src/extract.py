import json, sqlite3, sys
import os, shutil, tempfile
# reads a brain's data dir (read-only: the database is copied first) into the work file the pictures use
SRC=sys.argv[1].rstrip('/')+'/'
WORK=os.environ.get('README_WORK',os.path.join(tempfile.gettempdir(),'anamnesis-readme'))+'/'
os.makedirs(WORK,exist_ok=True)
for f in os.listdir(SRC):
    if f.startswith('brain.sqlite'): shutil.copy(SRC+f,WORK+f)
v=json.load(open(SRC+'views.json'))
m=json.load(open(SRC+'mind_state.json'))
c=sqlite3.connect(WORK+'brain.sqlite')
items={r[0]:(r[1],r[2]) for r in c.execute('select id,created_at,external_id from items')}
reg={}
regs=[]
for i,r in enumerate(m['regions']['regions']):
    regs.append({'label':r['region']['label'],'anatomy':r['region']['anatomy'],'size':r['region']['size']})
    for mem in r['members']: reg[mem]=i
links=[(a,b,t) for a,b,t in c.execute('select from_id,to_id,created_at from links')]
out=[]
for k,id_ in enumerate(v['ids']):
    if id_ not in items: continue
    ca,ext=items[id_]
    y=int(ca[:4]); mo=int(ca[5:7]); d=int(ca[8:10])
    if y>1900: continue
    sp='t' if ext.startswith('henry') else ('e' if ext.startswith('ralph') else '?')
    out.append({'id':id_,'a':v['anatomy'][k],'l':v['layout'][k],'s':v['spheres'][k],'r':reg.get(id_,-1),'t':y+(mo-1)/12+(d-1)/365,'sp':sp})
json.dump({'pts':out,'regions':regs,'links':links},open(WORK+'data.json','w'))
print(len(out), sum(1 for o in out if o['r']<0), len(links))
