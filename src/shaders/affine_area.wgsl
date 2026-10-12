struct Parameters {
 source:vec2u, output:vec2u, origin:vec2u, tile:vec2u,
 start:vec2f, dx:vec2f, dy:vec2f, padding:vec2f
}
struct Polygon { points:array<vec2f,8>, count:u32 }
@group(0) @binding(0) var image:texture_2d<f32>;
@group(0) @binding(1) var<storage,read_write> sums:array<vec4f>;
@group(0) @binding(2) var<uniform> p:Parameters;
fn cross(a:vec2f,b:vec2f)->f32 { return a.x*b.y-a.y*b.x; }
fn inside(v:vec2f,axis:u32,edge:f32,greater:bool)->bool {
 return select(v[axis]<=edge,v[axis]>=edge,greater);
}
fn clipped(poly:Polygon,axis:u32,edge:f32,greater:bool)->Polygon {
 var result:Polygon;
 if poly.count==0u { return result; }
 for(var i=0u;i<poly.count;i++) {
  let a=poly.points[(i+poly.count-1u)%poly.count]; let b=poly.points[i];
  let ia=inside(a,axis,edge,greater); let ib=inside(b,axis,edge,greater);
  if ia!=ib && a[axis]!=edge && b[axis]!=edge {
   var point=a+(b-a)*((edge-a[axis])/(b[axis]-a[axis])); point[axis]=edge;
   result.points[result.count]=point; result.count++;
  }
  if ib { result.points[result.count]=b; result.count++; }
 }
 return result;
}
fn overlap(start:vec2f,cell:vec2f)->f32 {
 // Test interior source cells first; reduced-scale reconstruction is mostly full cells.
 let determinant=cross(p.dx,p.dy);
 var full=true;
 for(var i=0u;i<4u;i++) {
  let corner=cell+array<vec2f,4>(vec2f(0.,0.),vec2f(1.,0.),vec2f(1.,1.),vec2f(0.,1.))[i]-start;
  let local=vec2f(cross(corner,p.dy),cross(p.dx,corner))/determinant;
  full=full && all(local>=vec2f(0.)) && all(local<=vec2f(1.));
 }
 if full { return 1.; }
 var poly:Polygon; poly.count=4u;
 poly.points[0]=start-cell; poly.points[1]=start+p.dx-cell;
 poly.points[2]=start+p.dx+p.dy-cell; poly.points[3]=start+p.dy-cell;
 poly=clipped(poly,0u,0.,true); poly=clipped(poly,0u,1.,false);
 poly=clipped(poly,1u,0.,true); poly=clipped(poly,1u,1.,false);
 if poly.count<3u { return 0.; }
 var area=0.;
 for(var i=1u;i+1u<poly.count;i++) {
  area+=cross(poly.points[i]-poly.points[0],poly.points[i+1u]-poly.points[0]);
 }
 return min(abs(area)*0.5,1.);
}
@compute @workgroup_size(8,8) fn main(@builtin(global_invocation_id) id:vec3u) {
 if any(id.xy>=p.output) { return; }
 let start=p.start+f32(id.x)*p.dx+f32(id.y)*p.dy;
 let points=array<vec2f,4>(start,start+p.dx,start+p.dx+p.dy,start+p.dy);
 var low=points[0]; var high=low;
 for(var i=1u;i<4u;i++) { low=min(low,points[i]); high=max(high,points[i]); }
 let lo=max(vec2i(floor(low)),vec2i(p.origin));
 let hi=min(min(vec2i(ceil(high)),vec2i(p.origin+p.tile)),vec2i(p.source));
 if any(lo>=hi) { return; }
 var sum=vec3f(0.);
 for(var y=lo.y;y<hi.y;y++) { for(var x=lo.x;x<hi.x;x++) {
  let weight=overlap(start,vec2f(f32(x),f32(y)));
  if weight>0. { sum+=textureLoad(image,vec2i(x,y)-vec2i(p.origin),0).rgb*weight; }
 } }
 let index=id.x+id.y*p.output.x;
 sums[index]+=vec4f(sum/abs(cross(p.dx,p.dy)),0.);
}
