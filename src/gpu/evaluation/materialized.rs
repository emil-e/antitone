//! Stored inputs selected during planning, without replacing the calculation graph.
use super::CellRegion as Rect;
use super::materializations::ImageStorage;
use super::*;

#[cfg(all(test, feature = "editor"))]
mod tests;

pub(super) const MAX_PARTS: usize = 4;

fn overlap(a: Rect, b: Rect) -> u64 {
    let width = (i64::from(a.right().min(b.right())) - i64::from(a.x.max(b.x))).max(0) as u64;
    let height = (i64::from(a.bottom().min(b.bottom())) - i64::from(a.y.max(b.y))).max(0) as u64;
    width * height
}

pub(super) struct Stored {
    parts: Vec<Part>,
}

struct Part {
    layout: ImageLayout,
    image: Arc<ImageValue>,
}

pub(super) fn validate(
    domain: Domain,
    layout: ImageLayout,
    image: &ImageValue,
) -> Result<(), Error> {
    layout.region.validate()?;
    if layout.domain != domain || !domain.contains_region(layout.region) {
        return Err(Error::InvalidParameter(
            "materialized storage belongs to a different image domain",
        ));
    }
    match (layout.backing, &image.storage) {
        (ImageBacking::Host, ImageStorage::Host(pixels))
            if layout.format == ImageFormat::Rgba32Float
                && pixels.len() as u64 * 16 == layout.bytes() => {}
        (ImageBacking::Buffer, ImageStorage::Buffer(pixels))
            if pixels.size() == layout.bytes()
                && pixels.usage().contains(wgpu::BufferUsages::STORAGE) => {}
        (ImageBacking::Texture, ImageStorage::Texture(pixels))
            if pixels.width() == layout.region.width
                && pixels.height() == layout.region.height
                && pixels.depth_or_array_layers() == 1
                && pixels.mip_level_count() == 1
                && pixels.sample_count() == 1
                && pixels
                    .usage()
                    .contains(wgpu::TextureUsages::TEXTURE_BINDING)
                && pixels.format()
                    == match layout.format {
                        ImageFormat::Rgba16Uint => wgpu::TextureFormat::Rgba16Uint,
                        ImageFormat::Rgba32Float => wgpu::TextureFormat::Rgba32Float,
                    } => {}
        _ => {
            return Err(Error::InvalidParameter(
                "materialized storage differs from its declared layout",
            ));
        }
    }
    Ok(())
}

impl Stored {
    pub(super) fn find(
        scope: Option<&Materializations>,
        supplied: &[SuppliedImage],
        definition: &value::Value,
        region: Rect,
    ) -> Result<Option<Self>, Error> {
        let mut values: Vec<Part> = supplied
            .iter()
            .filter(|image| image.supplies(definition))
            .map(|image| Part {
                layout: image.layout,
                image: image.image.clone(),
            })
            .collect();
        let output = definition.output(region)?;
        if let Some((scope, identity)) = scope.zip(output.identity()) {
            values.extend(
                scope
                    .ready::<ImageLayout>(identity)
                    .into_iter()
                    .filter(|image| image.descriptor.backing != ImageBacking::Host)
                    .map(|image| Part {
                        layout: *image.descriptor,
                        image: image.image,
                    }),
            );
        }
        for value in &values {
            validate(definition.domain, value.layout, &value.image)?;
        }
        values.sort_by_key(|value| {
            std::cmp::Reverse(
                u64::from(value.layout.region.width) * u64::from(value.layout.region.height),
            )
        });
        if let Some(index) = values
            .iter()
            .position(|value| value.layout.region.contains_region(region))
        {
            return Ok(Some(Self {
                parts: vec![values.remove(index)],
            }));
        }
        let mut parts: Vec<Part> = Vec::new();
        let mut covered = 0;
        for value in values {
            let r = value.layout.region;
            let contribution = overlap(r, region);
            if contribution == 0 || parts.iter().any(|part| overlap(r, part.layout.region) > 0) {
                continue;
            }
            covered += contribution;
            parts.push(value);
            if covered == u64::from(region.width) * u64::from(region.height) {
                parts.sort_by_key(|part| (part.layout.region.y, part.layout.region.x));
                return Ok(Some(Self { parts }));
            }
            if parts.len() == MAX_PARTS {
                break;
            }
        }
        // A partial or incompatible set is a cache miss, not an incomplete image.
        Ok(None)
    }

    pub(super) fn raster(
        &self,
        f: &mut Fragment,
        domain: Domain,
        region: Rect,
        identity: Option<&RetainedKey>,
    ) -> Option<PlannedImage> {
        let [part] = self.parts.as_slice() else {
            return None;
        };
        if part.layout.region != region
            || part.layout.format != ImageFormat::Rgba32Float
            || !matches!(part.image.storage, ImageStorage::Buffer(_))
        {
            return None;
        }
        Some(PlannedImage {
            domain,
            region,
            resource: f.resource(match identity {
                Some(key) => Storage::Image {
                    key: key.clone(),
                    layout: part.layout,
                    pixels: Some(part.image.clone()),
                },
                None => Storage::ResidentBuffer(part.image.clone()),
            }),
        })
    }

    pub(super) fn sample(&self, f: &mut Fragment) -> Sampled {
        if let [part] = self.parts.as_slice()
            && part.layout.format == ImageFormat::Rgba32Float
            && matches!(part.image.storage, ImageStorage::Buffer(_))
        {
            return Sampled::buffer(f, part.layout.region, Binding::Shared(part.image.binding()));
        }
        let mut code = format!(
            "@group(0) @binding(48) var<uniform> materialized_regions: array<vec4u,{}>;\n",
            self.parts.len()
        );
        let mut branches = String::new();
        let mut bindings = Vec::new();
        let mut rectangles = Vec::new();
        for (i, part) in self.parts.iter().enumerate() {
            let layout = part.layout;
            let r = layout.region;
            rectangles.extend_from_slice(&r.packed());
            let slot = 40 + i as u32;
            let name = format!("materialized_part_{i}");
            let sample = match &part.image.storage {
                ImageStorage::Host(_) => unreachable!("host values are not resident GPU inputs"),
                ImageStorage::Buffer(_) => {
                    let element = match layout.format {
                        ImageFormat::Rgba16Uint => "vec2u",
                        ImageFormat::Rgba32Float => "vec4f",
                    };
                    code.push_str(&format!(
                        "@group(0) @binding({slot}) var<storage,read> {name}: array<{element}>;\n"
                    ));
                    bindings.push((slot, Binding::Shared(part.image.binding())));
                    let load = format!(
                        "{name}[u32(local.y) * materialized_regions[{i}].z + u32(local.x)]"
                    );
                    match layout.format {
                        ImageFormat::Rgba32Float => format!("return {load};"),
                        ImageFormat::Rgba16Uint => format!(
                            "let code={load}; return vec4f(f32(code.x & 65535u),f32(code.x >> 16u),f32(code.y & 65535u),f32(code.y >> 16u));"
                        ),
                    }
                }
                ImageStorage::Texture(_) => {
                    let component = match layout.format {
                        ImageFormat::Rgba16Uint => "u32",
                        ImageFormat::Rgba32Float => "f32",
                    };
                    code.push_str(&format!(
                        "@group(0) @binding({slot}) var {name}: texture_2d<{component}>;\n"
                    ));
                    bindings.push((slot, Binding::Shared(part.image.binding())));
                    format!("return vec4f(textureLoad({name},local,0));")
                }
            };
            branches.push_str(&format!("if all(p >= vec2i(materialized_regions[{i}].xy)) && all(p < vec2i(materialized_regions[{i}].xy + materialized_regions[{i}].zw)) {{ let local=p-vec2i(materialized_regions[{i}].xy); {sample} }}\n"));
        }
        code.push_str(&format!(
            "fn sample_materialized(p:vec2i) -> vec4f {{ {branches} return vec4f(0.,0.,0.,-1.); }}"
        ));
        bindings.push((
            48,
            Binding::Uniform(
                f.resource(Storage::Uniform(bytemuck::cast_slice(&rectangles).into())),
            ),
        ));
        Sampled {
            code: code.into(),
            symbol: "sample_materialized".into(),
            bindings,
        }
    }
}
