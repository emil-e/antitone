use antitone::{gpu::{ProcessingKernel,ProcessingParameters},processing::{Calibration,Pipeline,RollSettings,PhotoSettings},color::Balance,rendering::{CpuRenderer,OutputTarget},viewer::GpuImage};
use glam::DVec3;
use std::{error::Error,fs};
use wgpu::util::DeviceExt;
type Result<T>=std::result::Result<T,Box<dyn Error>>;
fn decode(bytes:&[u8])->Vec<[f32;4]> {bytes.chunks_exact(16).map(|p|std::array::from_fn(|c|f32::from_le_bytes(p[c*4..c*4+4].try_into().unwrap()))).collect()}
fn map(device:&wgpu::Device,buffer:&wgpu::Buffer)->Result<Vec<u8>> {
    let(tx,rx)=std::sync::mpsc::channel();buffer.map_async(wgpu::MapMode::Read,..,move|r|tx.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely())?;rx.recv()??;
    let mapped=buffer.get_mapped_range(..)?;let bytes=mapped.to_vec();drop(mapped);buffer.unmap();Ok(bytes)
}
fn main()->Result<()> {pollster::block_on(async {
    let dir=".cache/probes/dmin-trace/run2";
    let numbers:Vec<f64>=fs::read_to_string(format!("{dir}/whole-dmin.txt"))?.split_whitespace().map(str::parse).collect::<std::result::Result<_,_>>()?;
    let whole=DVec3::new(numbers[0],numbers[1],numbers[2]);
    let border_values:Vec<f64>=fs::read_to_string(".cache/probes/dmin-trace/run1/verified-dmin.txt")?.split_whitespace().map(str::parse).collect::<std::result::Result<_,_>>()?;
    let border=DVec3::new(border_values[0],border_values[1],border_values[2]);
    let old=DVec3::new(0.140103,0.113156,0.102145);
    let data=fs::read(format!("{dir}/fur-scan-ap0-f64.bin"))?;
    let mut scans:Vec<DVec3>=data.chunks_exact(24).map(|p|DVec3::from_array(std::array::from_fn(|c|f64::from_le_bytes(p[c*8..c*8+8].try_into().unwrap())))).collect();
    for b in [old,border,whole] {for delta in [-1e-3,-1e-5,-1e-7,0.,1e-7,1e-5,1e-3] {scans.push(b*(1.+delta));}}
    for row in fs::read_to_string(format!("{dir}/representative-stage-traces.csv"))?.lines().skip(1) {
        let fields:Vec<_>=row.split(',').collect();scans.push(DVec3::new(fields[7].parse()?,fields[8].parse()?,fields[9].parse()?));
    }
    let pixels:Vec<[f32;4]>=scans.iter().map(|v|v.as_vec3().extend(0.).to_array()).collect();
    let mut bytes=Vec::new();for p in &pixels {for v in p {bytes.extend(v.to_le_bytes());}}
    let instance=wgpu::Instance::new(wgpu::InstanceDescriptor{backends:wgpu::Backends::METAL,..wgpu::InstanceDescriptor::new_without_display_handle()});
    let adapter=instance.request_adapter(&Default::default()).await?;
    println!("HEADLESS DIAGNOSTIC ONLY adapter={:?} scans={} (3072 fur samples plus boundary/representative traces)",adapter.get_info(),scans.len());
    let(device,queue)=adapter.request_device(&Default::default()).await?;
    let kernel=ProcessingKernel::new(&device).await?;
    let input=device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:None,contents:&bytes,usage:wgpu::BufferUsages::STORAGE});
    let output=device.create_buffer(&wgpu::BufferDescriptor{label:None,size:bytes.len() as u64,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let working_read=device.create_buffer(&wgpu::BufferDescriptor{label:None,size:output.size(),usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
    let image=GpuImage::new(&device,&queue,pixels.len() as u32,1,&pixels,OutputTarget::DisplayP3).await?;
    let row=(pixels.len() as u32*16).div_ceil(256)*256;
    let target_read=device.create_buffer(&wgpu::BufferDescriptor{label:None,size:row as u64,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
    for(name,b,ev) in [("screenshot-Dmin",old,3.),("clear-margin-Dmin",border,3.),("whole-picture-Dmin",whole,3.),("whole-picture-Dmin-plus2EV",whole,5.)] {
        let cal=Calibration{dmin:b,dmax:DVec3::new(0.008061,0.003409,0.001442),input_floor:numbers[3]};
        let roll=RollSettings{gamma:0.65,balance:Balance{warmth:-0.41,tint:-0.01},exposure:ev};
        let photo=PhotoSettings::default();let pipe=Pipeline::new(cal,roll,photo)?;
        let params=ProcessingParameters::positive(cal,roll,photo)?;
        let mut enc=device.create_command_encoder(&Default::default());
        kernel.encode(&device,&mut enc,&input,&output,params,pixels.len() as u32);
        enc.copy_buffer_to_buffer(&output,0,&working_read,0,output.size());
        image.encode(&device,&mut enc,params);
        enc.copy_texture_to_buffer(image.linear.as_image_copy(),wgpu::TexelCopyBufferInfo{buffer:&target_read,layout:wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(row),rows_per_image:Some(1)}},image.linear.size());
        queue.submit([enc.finish()]);
        let working=decode(&map(&device,&working_read)?);
        let target_bytes=map(&device,&target_read)?;let target=decode(&target_bytes[..bytes.len()]);
        let expected=scans.iter().map(|s|Ok(pipe.apply(*s)?.rgb)).collect::<Result<Vec<_>>>()?;
        let cpu=CpuRenderer::new(OutputTarget::DisplayP3)?.render(&expected)?;
        let mut working_error=0_f64;let mut target_error=0_f32;let mut encoded_error=0_f64;let mut extra_black=0;let mut extra_max_code=0_f64;
        for i in 0..scans.len() {
            for c in 0..3 {
                working_error=working_error.max((working[i][c] as f64-expected[i][c]).abs());
                target_error=target_error.max((target[i][c]-cpu[i][c]).abs());
                encoded_error=encoded_error.max((OutputTarget::DisplayP3.encode(target[i][c] as f64)?-OutputTarget::DisplayP3.encode(cpu[i][c] as f64)?).abs());
            }
            assert_eq!(working[i][3],0.);
            if target[i][..3].iter().all(|v|*v==0.)&&cpu[i].max_element()>0. {extra_black+=1;extra_max_code=extra_max_code.max(OutputTarget::DisplayP3.encode(cpu[i].max_element() as f64)?);}
        }
        println!("CASE {name} full_f64_source_CPU_vs_GPU working_max_abs={working_error:.9e} target_linear_max_abs={target_error:.9e} target_encoded_max_abs={encoded_error:.9e} GPU_black_CPU_nonblack={extra_black} largest_CPU_code_for_extra_GPU_black={extra_max_code:.9e}");
        assert!(working_error<2e-5&&target_error<2e-5&&extra_max_code<1./255.);
    }
    Ok(())
})}
