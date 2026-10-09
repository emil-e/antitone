use antitone::{gpu::ProcessingParameters, processing::{Calibration, PhotoSettings, Pipeline, RollSettings}, rendering::{CpuRenderer, OutputTarget}, viewer::GpuImage};
use glam::DVec3;

async fn read(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<[f32;4]> {
    let row = (texture.width() * 16).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {label: None, size: row as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation:false});
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(texture.as_image_copy(), wgpu::TexelCopyBufferInfo {buffer: &buffer, layout:wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(row),rows_per_image:Some(1)}},texture.size());
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read,..,move |result|tx.send(result).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();rx.recv().unwrap().unwrap();
    let mapped = buffer.get_mapped_range(..).unwrap();
    let out = mapped[..texture.width() as usize*16].chunks_exact(16).map(|p| std::array::from_fn(|c| f32::from_le_bytes(p[c*4..c*4+4].try_into().unwrap()))).collect();
    drop(mapped);buffer.unmap();out
}
fn main() {pollster::block_on(async {
    let cal = Calibration {dmin:DVec3::new(0.16512039312008006,0.13295850981449217,0.11888115148085542), dmax:DVec3::new(0.008358001578451848,0.003545242472549229,0.001551173905987176), input_floor:2.163093362227e-5};
    let roll=RollSettings::default();let photo=PhotoSettings::default();
    let pipe=Pipeline::new(cal,roll,photo).unwrap();
    let gamma=roll.gamma;
    let denom=(cal.dmin/cal.dmax).powf(1./gamma)-DVec3::ONE;
    let levels=[0.,1e-7,1e-6,1e-5,1e-4,0.001,0.003,0.01,0.03,0.1,0.18,1.];
    let mut pixels=Vec::new();
    for y in levels {let s=cal.dmin/(DVec3::ONE+denom*y).powf(gamma);pixels.push(s.as_vec3().extend(0.).to_array());}
    for delta in [-0.01,-0.001,-1e-4,-1e-5,0.,1e-5,1e-4,0.001,0.01,0.05,0.1,0.2] {pixels.push((cal.dmin*(1.-delta)).as_vec3().extend(0.).to_array());}
    let bytes=std::fs::read(".cache/probes/camera-scan-ap0-model2.f32").unwrap();
    for p in bytes.chunks_exact(16) {pixels.push(std::array::from_fn(|c|f32::from_le_bytes(p[c*4..c*4+4].try_into().unwrap())));}
    let expected:Vec<_>=pixels.iter().map(|p|pipe.apply(DVec3::new(p[0] as f64,p[1] as f64,p[2] as f64)).unwrap().rgb).collect();
    let instance=wgpu::Instance::new(wgpu::InstanceDescriptor{backends:wgpu::Backends::METAL,..wgpu::InstanceDescriptor::new_without_display_handle()});
    let adapter=instance.request_adapter(&Default::default()).await.unwrap();
    println!("DIAGNOSTIC references, default gamma/balance/exposure/WB; NOT user grade; adapter={:?}",adapter.get_info());
    let (device,queue)=adapter.request_device(&Default::default()).await.unwrap();
    for target in [OutputTarget::Srgb,OutputTarget::DisplayP3] {
        let image=GpuImage::new(&device,&queue,pixels.len() as u32,1,&pixels,target).await.unwrap();
        let mut encoder=device.create_command_encoder(&Default::default());image.encode(&device,&mut encoder,ProcessingParameters::positive(cal,roll,photo).unwrap());queue.submit([encoder.finish()]);
        let actual=read(&device,&queue,&image.linear).await;
        let cpu=CpuRenderer::new(target).unwrap().render(&expected).unwrap();
        let mut maxerr=0f32;let mut gpu_black_cpu_nonblack=0;
        for (i,(a,e)) in actual.iter().zip(&cpu).enumerate() {
            for c in 0..3 {maxerr=maxerr.max((a[c]-e[c]).abs());}
            gpu_black_cpu_nonblack+=usize::from(a[..3].iter().all(|v|*v==0.)&&e.max_element()>0.);
            if i<24 {println!("target={target:?} i={i} AP1={:?} CPU_linear={:?} GPU_linear={:?} CPU_max_encoded={:.9}",expected[i],e,&a[..3],target.encode(e.max_element() as f64).unwrap());}
        }
        println!("target={target:?} samples={} GPU_CPU_max_abs_error={maxerr:.9e} GPU_black_CPU_nonblack={gpu_black_cpu_nonblack}",actual.len());
    }
    println!("Neutral inversion rays: scan = diagnostic Dmin*(1-delta), no balance/exposure/WB edits; encoded target maxima:");
    let renderer=CpuRenderer::new(OutputTarget::DisplayP3).unwrap();
    for gamma in [0.65,1.,3.] {for delta in [0.01,0.05,0.1,0.2] {
        let pipe=Pipeline::new(cal,RollSettings{gamma,..Default::default()},photo).unwrap();
        let p=pipe.apply(cal.dmin*(1.-delta)).unwrap().rgb;
        let output=renderer.render(&[p]).unwrap()[0];
        println!("gamma={gamma} delta={delta} AP1={p:?} linear={output:?} max_encoded={:.9}",OutputTarget::DisplayP3.encode(output.max_element() as f64).unwrap());
    }}
});}
