// Integer-only authoritative lower-bound sum. The float expression only decides
// when to stop accumulating: any returned prefix remains a lower bound, even if
// that stopping hint is inaccurate. CPU f64 verifies the integer bound before pruning.
@group(0) @binding(0) var<storage, read> features: array<u32>;
@group(0) @binding(1) var<storage, read> weights: array<u32>;
@group(0) @binding(2) var<storage, read> candidates: array<vec2u>;
@group(0) @binding(3) var<storage, read_write> results: array<vec4u>;
struct Params { width:u32, tx:u32, ty:u32, local_width:u32, cells:u32, count:u32, color_factor:f32, gradient_factor:f32, ceiling:f32, paired_weights:u32, upper_color:u32, upper_gradient:u32 }
@group(0) @binding(4) var<uniform> p:Params;
var<workgroup> sums:array<vec4u,128>;
var<workgroup> coverage:array<vec2u,128>;
var<workgroup> halt:u32;
fn add64(a:vec2u,b:vec2u)->vec2u {
    let low=a.x+b.x;
    return vec2u(low,a.y+b.y+select(0u,1u,low<a.x));
}
fn add_pair(a:vec4u,b:vec4u)->vec4u {return vec4u(add64(a.xy,b.xy),add64(a.zw,b.zw));}
fn approximate(a:vec2u)->f32 {return f32(a.y)*4294967296.0+f32(a.x);}
fn quant(pixel:u32,channel:u32)->i32 {
    let word=features[pixel*5u+channel/2u];
    let bits=(word>>(16u*(channel&1u)))&65535u;
    return i32(bits^32768u)-32768;
}
fn term(a:i32,b:i32,w:u32)->u32 {
    if a== -32768 || b== -32768 {return 0u;}
    let gap=u32(clamp(abs(a-b)-3,0,4095));
    return gap*gap*w; // <=4095^2*256 <2^32; saturation only weakens the screening bound.
}
@compute @workgroup_size(128)
fn main(@builtin(workgroup_id) group:vec3u,@builtin(local_invocation_index) lane:u32) {
    let c=group.y*1024u+group.x;
    if c>=p.count {return;}
    let origin=candidates[c];
    // Full paired-weight upper bounds: sums of dyadic ceilings are exact and
    // dominate the CPU's same-order positive paired-weight accumulation.
    if p.paired_weights!=0u {
        var paired=vec2u(0u);
        for(var i=lane;i<p.cells;i+=128u) {
            let upper=weights[i]>>9u;
            if upper==0u {continue;}
            let x=i%p.local_width;let y=i/p.local_width;
            let t=(p.ty+y)*p.width+p.tx+x;
            let s=(origin.y+y)*p.width+origin.x+x;
            let sf=features[s*5u+4u]>>16u;
            let tf=features[t*5u+4u]>>16u;
            if (sf&tf&1u)!=0u {paired.x+=upper;}
            if (sf&tf&2u)!=0u {paired.y+=upper;}
        }
        coverage[lane]=paired;workgroupBarrier();
        for(var step=64u;step>0u;step/=2u) {
            if lane<step {coverage[lane]+=coverage[lane+step];}
            workgroupBarrier();
        }
    }else {
        if lane==0u {coverage[0]=vec2u(p.upper_color,p.upper_gradient);}
        workgroupBarrier();
    }
    var sum=vec4u(0u);var processed=0u;
    for(var base=0u;base<p.cells;base+=2048u) {
        let end=min(base+2048u,p.cells);
        for(var i=base+lane;i<end;i+=128u) {
            let weight=weights[i]&511u;
            if weight==0u {continue;}
            let x=i%p.local_width;let y=i/p.local_width;
            let t=(p.ty+y)*p.width+p.tx+x;
            let s=(origin.y+y)*p.width+origin.x+x;
            let sf=features[s*5u+4u]>>16u;
            let tf=features[t*5u+4u]>>16u;
            if (sf&tf&1u)!=0u {
                for(var ch=0u;ch<3u;ch++) {
                    sum=vec4u(add64(sum.xy,vec2u(term(quant(t,ch),quant(s,ch),weight),0u)),sum.zw);
                }
            }
            if (sf&tf&2u)!=0u {
                for(var ch=3u;ch<9u;ch++) {
                    sum=vec4u(sum.xy,add64(sum.zw,vec2u(term(quant(t,ch),quant(s,ch),weight),0u)));
                }
            }
        }
        sums[lane]=sum;workgroupBarrier();
        for(var step=64u;step>0u;step/=2u) {
            if lane<step {sums[lane]=add_pair(sums[lane],sums[lane+step]);}
            workgroupBarrier();
        }
        if lane==0u {
            halt=select(0u,1u,approximate(sums[0].xy)*p.color_factor+approximate(sums[0].zw)*p.gradient_factor>p.ceiling*1.001);
        }
        workgroupBarrier();processed=end;
        if halt!=0u {break;}
    }
    if lane==0u {results[c*2u]=sums[0];results[c*2u+1u]=vec4u(processed,coverage[0].x,coverage[0].y,0u);}
}
