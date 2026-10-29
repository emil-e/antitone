//! Resident GPU input is a source component, not another execution loop.
use super::*;
use crate::geometry::SourceRegion as Rect;
#[cfg(test)]
pub(crate) fn raster_input(domain: Domain) -> Arc<ImageNode> {
    ImageNode::new(
        domain,
        None,
        Resident {
            domain,
            writable: true,
        },
    )
}
pub(crate) fn borrowed_input(domain: Domain) -> Arc<ImageNode> {
    ImageNode::new(
        domain,
        None,
        Resident {
            domain,
            writable: false,
        },
    )
}
struct Resident {
    domain: Domain,
    writable: bool,
}
impl Operation for Resident {
    fn input(&self, _: Rect) -> Result<Option<Rect>, Error> {
        Ok(None)
    }
    fn lower(
        &self,
        f: &mut Fragment,
        _: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        if output.region == self.domain.bounds {
            f.resources[output.resource.0] = Storage::Input {
                slot: 0,
                bytes: u64::from(output.cells()) * 16,
                writable: self.writable,
            };
            return Ok(());
        }
        let source = f.uniform(&[
            self.domain.bounds.x,
            self.domain.bounds.y,
            self.domain.bounds.width,
            self.domain.bounds.height,
        ]);
        let rectangle = f.uniform(&[
            output.region.x,
            output.region.y,
            output.region.width,
            output.region.height,
        ]);
        let pixels = f.resource(Storage::Input {
            slot: 0,
            bytes: u64::from(self.domain.bounds.width) * u64::from(self.domain.bounds.height) * 16,
            writable: self.writable,
        });
        f.compute(r#"
@group(0) @binding(0) var<storage,read> resident_pixels:array<vec4f>;
@group(0) @binding(1) var<storage,read_write> resident_result:array<vec4f>;
@group(0) @binding(2) var<uniform> resident_source:vec4u;
@group(0) @binding(3) var<uniform> resident_region:vec4u;
@compute @workgroup_size(64)
fn resident_main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u){
    let index = id.x + id.y * groups.x * 64u;
    if index>=resident_region.z*resident_region.w{return;}
    let p=resident_region.xy+vec2u(index%resident_region.z,index/resident_region.z)-resident_source.xy;
    resident_result[index]=resident_pixels[p.y*resident_source.z+p.x];
}
"#,"resident_main",vec![
            (0,Binding::Read(pixels)),
            (1,Binding::Write(output.resource)),(2,source),(3,rectangle),
        ],output.cells());
        Ok(())
    }
}
