"""Extract the characteristic-curve vectors in two public Kodak PDFs.

This is deliberately specific to the inspected page-4 streams, not a general PDF
parser. Axis bounds and curve block identifiers are recorded below and verified
against the original rendered pages. Data are published drawing coordinates,
not raw laboratory measurements.
"""
from pathlib import Path
import csv, hashlib, json, math, re, zlib
ROOT = Path('.cache/probes/film-response-docs')
CUBIC_STEPS = 256
SOURCES = {
 'portra400': dict(file='e4050_portra_400.pdf', origin=(101.814,544.606), offsets=(0,-10.173,-29.86), axis=(81.163,265.612,504.632,689.147), exposure=(-4,1)),
 'ektar100': dict(file='e4046_ektar_100.pdf', origin=(87.101,538.535), offsets=(0,-9.689,-29.313), axis=(81.193,265.698,499.592,684.076), exposure=(-3,2)),
}
def content(data):
    match = re.search(rb'\b8 0 obj\b(.*?)endobj',data,re.S)
    assert match
    stream = re.search(rb'stream\r?\n(.*?)\r?\nendstream',match.group(1),re.S)
    assert stream
    return zlib.decompress(stream.group(1)).decode('latin1')
def point(x,y,source):
    left,right,bottom,top=source['axis'];lo,hi=source['exposure']
    return lo+(x-left)/(right-left)*(hi-lo),(y-bottom)/(top-bottom)*4
metadata={}
for stock,source in SOURCES.items():
    data=(ROOT/source['file']).read_bytes(); text=content(data)
    ox,oy=source['origin']
    marker=f'q 1 0 0 1 {ox:g} {oy:g} cm 0 0 m'
    start=text.index(marker)+len(marker)
    end=text.index('\nQ',start)
    block=text[start:end]
    parts=re.split(r'S\s+1 0 0 1 0 (-?[\d.]+) cm 0 0 m',block)
    assert len(parts)==5
    chunks=[parts[0],parts[2],parts[4]]
    offsets=[0,float(parts[1]),float(parts[1])+float(parts[3])]
    assert all(abs(a-b)<1e-10 for a,b in zip(offsets,source['offsets']))
    raw=[]; uniform=[]
    for channel,chunk,offset in zip('BGR',chunks,offsets):
        path=[(0.,0.)]
        for line in chunk.splitlines():
            tokens=line.split()
            if not tokens:continue
            op=tokens[-1]
            if op not in ('l','c'):continue
            values=list(map(float,tokens[:-1]))
            if op=='l':
                assert len(values)==2
                path.append(tuple(values))
            else:
                assert len(values)==6
                p0=path[-1];p1=values[:2];p2=values[2:4];p3=values[4:]
                # Subdivision only recovers the publisher's drawn cubic; it adds no evidence.
                for i in range(1,CUBIC_STEPS+1):
                    t=i/CUBIC_STEPS;u=1-t
                    path.append(tuple(u*u*u*p0[c]+3*u*u*t*p1[c]+3*u*t*t*p2[c]+t*t*t*p3[c] for c in (0,1)))
        coords=[point(ox+x,oy+offset+y,source) for x,y in path]
        assert all(a[0]<b[0] for a,b in zip(coords,coords[1:])), channel
        for x,y in coords:raw.append((stock,channel,x,y))
        # Equal exposure spacing avoids overweighting the extra toe vertices.
        j=0
        for i in range(161):
            x=coords[0][0]+(coords[-1][0]-coords[0][0])*i/160
            while j+1<len(coords)-1 and coords[j+1][0]<x:j+=1
            a,b=coords[j:j+2]
            y=a[1]+(x-a[0])/(b[0]-a[0])*(b[1]-a[1])
            uniform.append((stock,channel,x,y))
        print(stock,channel,'drawn_trace_points',len(coords),'range',coords[0],coords[-1])
    for label,rows in [('trace',raw),('uniform',uniform)]:
        with (ROOT/f'{stock}-{label}.csv').open('w',newline='') as f:
            writer=csv.writer(f);writer.writerow(['stock','channel','log10_exposure','density']);writer.writerows(rows)
    metadata[stock]={**source,'sha256':hashlib.sha256(data).hexdigest(),'page':4,'stream_object':8,'curve_line_width_pdf_points':0.96,'cubic_subdivisions':CUBIC_STEPS,'line_width_density':0.96/(source['axis'][3]-source['axis'][2])*4,'resampling_note':'161 equally spaced exposure positions per trace; interpolated points are not independent measurements'}
(ROOT/'extraction.json').write_text(json.dumps(metadata,indent=2)+'\n')
