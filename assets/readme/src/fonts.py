import io, base64, sys
from fontTools.ttLib import TTFont
from fontTools import subset
from fontTools.varLib import instancer
import os
_R=os.path.abspath(os.path.join(os.path.dirname(__file__),'..','..','..'))+'/'
# subsets of the two faces, pinned to one instance, so each picture carries only the glyphs it draws
A=_R+'ui/theme/themes/aphesis/assets/AphesisScript-VF.woff2'
S=_R+'ui/theme/fonts/SpecialElite-Regular.woff2'
def _sub(path, text, pin=None, feats=('kern','calt','liga')):
    f=TTFont(path)
    if pin and 'fvar' in f: f=instancer.instantiateVariableFont(f, pin)
    o=subset.Options(); o.flavor='woff2'; o.layout_features=list(feats); o.name_IDs=[0,1,2,3,4,5,6]; o.notdef_outline=False; o.hinting=False
    s=subset.Subsetter(o); s.populate(text=text); s.subset(f)
    b=io.BytesIO(); f.flavor='woff2'; f.save(b); return b.getvalue()
def aph(text, pin={'wght':400,'slnt':-12}):
    return _sub(A, text, pin)
def elite(text):
    return _sub(S, text)
def face(name, data, extra=''):
    return "@font-face{font-family:'%s';src:url(data:font/woff2;base64,%s) format('woff2');%s}" % (name, base64.b64encode(data).decode(), extra)
if __name__=='__main__':
    print(len(aph('anamnesis god soul ice snow moon night')), len(aph('anamnesis god soul ice snow moon night',pin=None)), len(elite('abcdefghijklmnopqrstuvwxyz 0123456789')))
