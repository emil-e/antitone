//! An independently supplied finite-neighbourhood component used by composition tests.
use crate::gpu::pull::{self, Binding, Domain, Fragment, Image, Operation};
use crate::{
    Error, geometry::SourceRegion as Rect, pixel_pipeline::Layer, processing_support::Support,
};
use std::sync::Arc;
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NeighbourLayer {
    pub radius: f64,
    pub gain: f32,
}
impl Layer for NeighbourLayer {
    fn support(&self, pitch: [f64; 2]) -> Support {
        Support([(self.radius / pitch[0]).ceil() as u32, 0])
    }
    fn compose(&self, input: Arc<pull::Layer>) -> Result<Arc<pull::Layer>, Error> {
        let neighbourhood = pull::Layer::new(
            input.domain,
            Some(input.clone()),
            Neighbour {
                domain: input.domain,
                margin: self.support(input.domain.grid.pitch),
                gain: self.gain,
            },
        );
        Ok(pull::pointwise(
            neighbourhood,
            input.domain.meaning,
            pull::Function {
                code: r#"fn independent_source_bias(pixel:vec4f,position:vec2f)->vec4f{
                return vec4f(pixel.rgb+vec3f(position.x*0.000001+position.y*0.000002),pixel.w);
            }"#
                .into(),
                symbol: "independent_source_bias",
                bindings: vec![],
            },
        ))
    }
}
struct Neighbour {
    domain: Domain,
    margin: Support,
    gain: f32,
}
impl Operation for Neighbour {
    fn input(&self, output: Rect) -> Result<Option<Rect>, Error> {
        Ok(Some(self.margin.expand(output, self.domain.bounds)))
    }
    fn lower(&self, f: &mut Fragment, input: Option<Image>, output: Image) -> Result<(), Error> {
        let input = input.unwrap();
        let geometry = f.uniform(&[
            [
                input.region.x as f32,
                input.region.y as f32,
                input.region.width as f32,
                input.region.height as f32,
            ],
            [
                output.region.x as f32,
                output.region.y as f32,
                output.region.width as f32,
                output.region.height as f32,
            ],
            [self.margin.0[0] as f32, self.gain, 0., 0.],
        ]);
        f.compute(r#"
@group(0) @binding(0) var<storage,read> neighbour_input:array<vec4f>;
@group(0) @binding(1) var<storage,read_write> neighbour_output:array<vec4f>;
struct NeighbourGeometry {input:vec4f,output:vec4f,values:vec4f}
@group(0) @binding(2) var<uniform> neighbour_geometry:NeighbourGeometry;
fn neighbour_at(p:vec2i)->vec4f {
    let local=clamp(p-vec2i(neighbour_geometry.input.xy),vec2i(0),vec2i(neighbour_geometry.input.zw)-vec2i(1));
    return neighbour_input[u32(local.y)*u32(neighbour_geometry.input.z)+u32(local.x)];
}
@compute @workgroup_size(64)
fn neighbour_main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u){
    let index = id.x + id.y * groups.x * 64u;
    let size=vec2u(neighbour_geometry.output.zw);
    if index>=size.x*size.y{return;}
    let xy=vec2u(index%size.x,index/size.x);
    let p=vec2i(neighbour_geometry.output.xy)+vec2i(xy);
    let center=neighbour_at(p);
    let radius=i32(neighbour_geometry.values.x);
    let left=neighbour_at(p-vec2i(radius,0));
    let right=neighbour_at(p+vec2i(radius,0));
    neighbour_output[index]=vec4f((left.rgb+2.0*center.rgb+right.rgb)*0.25*neighbour_geometry.values.y,center.w);
}
"#,"neighbour_main",vec![(0,Binding::Read(input.resource)),(1,Binding::Write(output.resource)),(2,geometry)],output.cells());
        Ok(())
    }
}
