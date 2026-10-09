use antitone::{color::{ap0_to_ap1, Balance}, processing::{Calibration, Inversion, RollSettings}, raw::RawFile};
use glam::{DMat3, DVec3};
use std::{error::Error, fs, path::Path, time::Instant};
use wgpu::util::DeviceExt;
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const FLOOR: f64 = 2.1630933622265155e-5;
fn lr(a: f64,b: f64)->f64 { let r=(a-b)/b; if r.abs()<0.5 {r.ln_1p()} else {a.ln()-b.ln()} }
fn la(a: f64)->f64 { if a>0. {a+(-(-a).exp_m1()).ln()} else {(-a.exp_m1()).ln()} }
fn anchor(cal: Calibration,g:DVec3,beta:f64,k:f64)->DVec3 {DVec3::from_array(std::array::from_fn(|c| {let l=lr(cal.dmin[c],cal.dmax[c]); beta*l/(g[c]-k*l/std::f64::consts::LN_10)}))}
fn value(s:DVec3,cal:Calibration,g:DVec3,a:DVec3,beta:f64,k:f64,ev:f64,m:DMat3)->DVec3 {
 let q=DVec3::from_array(std::array::from_fn(|c| {let l=lr(cal.dmin[c],s[c].max(cal.input_floor)); let z=beta*l/(g[c]-k*l/std::f64::consts::LN_10); if z==0. {0.} else {z.signum()*((la(z)-la(a[c]))/beta+ev*std::f64::consts::LN_2).exp()}}));m*q
}
fn split(v: DVec3, w: f64)->[[f32;4];3] {
 let h=v.as_vec3(); let l=(v-h.as_dvec3()).as_vec3(); let t=(v-h.as_dvec3()-l.as_dvec3()).as_vec3();
 let wh=w as f32; let wl=(w-wh as f64) as f32; let wt=(w-wh as f64-wl as f64) as f32;
 [h.extend(wh).to_array(),l.extend(wl).to_array(),t.extend(wt).to_array()]
}
fn bytes(rows: &[[f32;4]])->Vec<u8> {rows.iter().flatten().flat_map(|v|v.to_le_bytes()).collect()}
fn camera_value(code: [u16;4])->DVec3 {DVec3::new(code[0]as f64,code[1]as f64,code[2]as f64)/65535.}
fn code_bytes(codes: &[[u16;4]])->Vec<u8> {codes.iter().flatten().flat_map(|v|v.to_le_bytes()).collect()}
fn packed_ap0(codes:&[[u16;4]],matrix:DMat3)->Vec<u8> {bytes(&codes.iter().flat_map(|c|split(matrix*camera_value(*c),0.)).collect::<Vec<_>>())}
fn params(matrix:DMat3,g:DVec3,beta:f64,ev:f64,m:DMat3)->Vec<u8> {
 let mut rows=vec![g.as_vec3().extend(beta as f32).to_array()];
 rows.extend([m.x_axis,m.y_axis,m.z_axis].map(|v|v.as_vec3().extend(0.).to_array()));
 rows.push([(ev*std::f64::consts::LN_2)as f32,0.,0.,0.]);
 let columns=[matrix.x_axis,matrix.y_axis,matrix.z_axis].map(|v|split(v,0.));
 for part in 0..3 {for col in columns {rows.push(col[part]);}}
 bytes(&rows)
}
fn refs(calibrations:&[Calibration],g:DVec3,beta:f64,k:f64)->Vec<u8> {
 let mut rows=Vec::new();for &cal in calibrations {rows.extend(split(cal.dmin,cal.input_floor)); rows.push(anchor(cal,g,beta,k).as_vec3().extend(k as f32).to_array());}bytes(&rows)
}
fn buffer(device:&wgpu::Device,data:&[u8],usage:wgpu::BufferUsages)->wgpu::Buffer {device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:None,contents:data,usage})}
fn group(device:&wgpu::Device,pipe:&wgpu::ComputePipeline,input:&wgpu::Buffer,out:&wgpu::Buffer,uniform:&wgpu::Buffer,reference:Option<&wgpu::Buffer>)->wgpu::BindGroup {
 let mut entries=vec![wgpu::BindGroupEntry{binding:0,resource:input.as_entire_binding()},wgpu::BindGroupEntry{binding:1,resource:out.as_entire_binding()},wgpu::BindGroupEntry{binding:2,resource:uniform.as_entire_binding()}];
 if let Some(reference)=reference {entries.push(wgpu::BindGroupEntry{binding:3,resource:reference.as_entire_binding()});}
 device.create_bind_group(&wgpu::BindGroupDescriptor{label:None,layout:&pipe.get_bind_group_layout(0),entries:&entries})
}
fn dispatch(encoder:&mut wgpu::CommandEncoder,pipe:&wgpu::ComputePipeline,group:&wgpu::BindGroup,n:u32) {
 let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(pipe);pass.set_bind_group(0,group,&[]);pass.dispatch_workgroups(n.div_ceil(64),1,1);
}
fn read(device:&wgpu::Device,queue:&wgpu::Queue,pipe:&wgpu::ComputePipeline,group:&wgpu::BindGroup,out:&wgpu::Buffer,n:u32,size:u64)->Result<Vec<[f32;4]>> {
 let read=device.create_buffer(&wgpu::BufferDescriptor{label:None,size,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
 let mut encoder=device.create_command_encoder(&Default::default());dispatch(&mut encoder,pipe,group,n);encoder.copy_buffer_to_buffer(out,0,&read,0,size);queue.submit([encoder.finish()]);
 let(tx,rx)=std::sync::mpsc::channel();read.map_async(wgpu::MapMode::Read,..,move|r|tx.send(r).unwrap());device.poll(wgpu::PollType::wait_indefinitely())?;rx.recv()??;
 let map=read.get_mapped_range(..)?;let result=map.chunks_exact(16).map(|v|std::array::from_fn(|c|f32::from_le_bytes(v[4*c..4*c+4].try_into().unwrap()))).collect();drop(map);read.unmap();Ok(result)
}
fn pipeline(device:&wgpu::Device,code:String,entry:&str)->wgpu::ComputePipeline {
 let module=device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some(entry),source:wgpu::ShaderSource::Wgsl(code.into())});
 device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor{label:Some(entry),layout:None,module:&module,entry_point:Some(entry),compilation_options:Default::default(),cache:None})
}
fn time_compute(device:&wgpu::Device,queue:&wgpu::Queue,pipe:&wgpu::ComputePipeline,bind:&wgpu::BindGroup,n:u32)->Result<Vec<f64>> {
 let mut timings=Vec::new();for batch in 0..7 {let started=Instant::now();let mut encoder=device.create_command_encoder(&Default::default());for _ in 0..5 {dispatch(&mut encoder,pipe,bind,n);}queue.submit([encoder.finish()]);device.poll(wgpu::PollType::wait_indefinitely())?;if batch>=2 {timings.push(started.elapsed().as_secs_f64()*1000./5.);}}Ok(timings)
}
fn median(values:&[f64])->f64 {let mut x=values.to_vec();x.sort_by(f64::total_cmp);x[x.len()/2]}
fn main()->Result<()> {pollster::block_on(async {
 let cal=Calibration{dmin:DVec3::new(0.16856635015466970,0.13359162596075455,0.11715123642832656),dmax:DVec3::new(0.008061,0.003409,0.001442),input_floor:FLOOR};
 let mut raw=RawFile::open(Path::new("samples/DSC04393.ARW"))?;let camera_matrix=raw.metadata.camera_to_ap0()?;let developed=raw.develop_camera([2133.,1024.,1771.,1024.])?;
 let mut codes=Vec::new();let mut calibrations=Vec::new();
 for (rx,ry) in [(5750,2400),(5100,3350),(6650,4200)] {for y in 0..32 {for x in 0..32 {
  let cam=developed.pixel(rx+(2*x+1)*512/64,ry+(2*y+1)*384/64)?;
  let code=[(cam.x*65535.).round()as u16,(cam.y*65535.).round()as u16,(cam.z*65535.).round()as u16,0];assert_eq!(camera_value(code),cam);codes.push(code);calibrations.push(cal);
 }}}
 let fur=codes.len();let boundary_code=[12695,8535,7802,0];let base=camera_matrix*camera_value(boundary_code);
 // Keep integer input exact; vary the calibration instead of inventing non-RGB16 source samples.
 for eps in [-1e-3,-1e-5,-1e-7,-1e-8,-1e-9,-1e-12,0.,1e-12,1e-9,1e-8,1e-7,1e-5,1e-3] {
  for channel in 0..4 {let mut b=base;if channel==3 {b*=1.+eps;}else{b[channel]*=1.+eps;}codes.push(boundary_code);calibrations.push(Calibration{dmin:b,..cal});}
 }
 for code in [[0,0,0,0],[65535,65535,65535,0],[0,65535,0,0],[65535,0,0,0],[0,0,65535,0],[1,2,2,0],[500,200,100,0]] {codes.push(code);calibrations.push(cal);}
 let anchor_code=[500,200,100,0];codes.push(anchor_code);calibrations.push(Calibration{dmax:camera_matrix*camera_value(anchor_code),..cal});
 let floor_code=[1,2,2,0];let near_floor=camera_matrix*camera_value(floor_code);for c in 0..3 {for eps in [-1e-12,0.,1e-12] {codes.push(floor_code);calibrations.push(Calibration{input_floor:near_floor[c]*(1.+eps),..cal});}}
 let scans:Vec<_>=codes.iter().map(|c|camera_matrix*camera_value(*c)).collect();
 let archived=fs::read(".cache/probes/dmin-trace/run2/fur-scan-ap0-f64.bin")?;assert_eq!(archived.len(),fur*24);
 for i in 0..fur {for c in 0..3 {assert_eq!(scans[i][c],f64::from_le_bytes(archived[24*i+8*c..24*i+8*c+8].try_into().unwrap()));}}
 let instance=wgpu::Instance::new(wgpu::InstanceDescriptor{backends:wgpu::Backends::METAL,..wgpu::InstanceDescriptor::new_without_display_handle()});let adapter=instance.request_adapter(&Default::default()).await?;let(device,queue)=adapter.request_device(&Default::default()).await?;
 println!("PACKED LIBRAW-DEVELOPED RGB16 PROBE adapter={:?} source_bytes_per_pixel=8 shader_temporary_only_extended_precision=true samples={} fur={} constructed_calibration_or_floor_cases={}",adapter.get_info(),codes.len(),fur,codes.len()-fur);
 let started=Instant::now();let shader=include_str!("../../camera-u16-precision.wgsl").to_owned();let pipe=pipeline(&device,shader.clone(),"main");let conversion=pipeline(&device,shader,"conversion_only");println!("compile_seconds={:.3}",started.elapsed().as_secs_f64());
 let input=buffer(&device,&code_bytes(&codes),wgpu::BufferUsages::STORAGE);
 let output=device.create_buffer(&wgpu::BufferDescriptor{label:None,size:(codes.len()*48)as u64,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
 let m=ap0_to_ap1();let roll=RollSettings{gamma:0.65,balance:Balance{warmth:-0.30045307385649744,tint:0.013735687288646154},exposure:0.};let g=roll.channel_gammas()?;
 let uniform=buffer(&device,&params(camera_matrix,g,2.5,3.,m),wgpu::BufferUsages::UNIFORM);let reference=buffer(&device,&refs(&calibrations,g,2.5,0.),wgpu::BufferUsages::STORAGE);
 let bind=group(&device,&conversion,&input,&output,&uniform,Some(&reference));let converted=read(&device,&queue,&conversion,&bind,&output,codes.len()as u32,(codes.len()*48)as u64)?;
 let mut conversion_bad=0;let mut high_bad=0;let mut conversion_max=0_f64;for(i,s)in scans.iter().enumerate(){for c in 0..3 {let actual=converted[3*i][c]as f64+converted[3*i+1][c]as f64+converted[3*i+2][c]as f64;conversion_max=conversion_max.max((actual-s[c]).abs());if converted[3*i][c].to_bits()!=(s[c]as f32).to_bits(){high_bad+=1;}if actual.to_bits()!=s[c].to_bits(){conversion_bad+=1;if conversion_bad<8 {println!("CONVERSION_MISMATCH index={i} channel={c} code={:?} actual={actual:.17e} CPU={:.17e} parts={:?}",codes[i],s[c],[converted[3*i][c],converted[3*i+1][c],converted[3*i+2][c]]);}}}}
 println!("SOURCE_CONVERSION f64_bit_mismatch_channels={conversion_bad} legacy_f32_source_bit_mismatch_channels={high_bad} maximum_absolute_error={conversion_max:.9e}");
 let mut stress:Vec<_>=(0..=65535u16).map(|v|[v,v,v,0]).collect();
 let mut rng=0x619af501u32;for _ in 0..65536 {let mut code=[0u16;4];for v in &mut code[..3] {rng^=rng<<13;rng^=rng>>17;rng^=rng<<5;*v=rng as u16;}stress.push(code);}
 let stress_input=buffer(&device,&code_bytes(&stress),wgpu::BufferUsages::STORAGE);
 let stress_output=device.create_buffer(&wgpu::BufferDescriptor{label:None,size:(stress.len()*48)as u64,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
 let stress_bind=group(&device,&conversion,&stress_input,&stress_output,&uniform,Some(&reference));
 let stress_gpu=read(&device,&queue,&conversion,&stress_bind,&stress_output,stress.len()as u32,(stress.len()*48)as u64)?;
 let mut stress_bad=0;let mut stress_high_bad=0;let mut stress_max=0_f64;for(i,code)in stress.iter().enumerate(){let expected=camera_matrix*camera_value(*code);for c in 0..3 {let actual=stress_gpu[3*i][c]as f64+stress_gpu[3*i+1][c]as f64+stress_gpu[3*i+2][c]as f64;stress_max=stress_max.max((actual-expected[c]).abs());if stress_gpu[3*i][c].to_bits()!=(expected[c]as f32).to_bits(){stress_high_bad+=1;}if actual.to_bits()!=expected[c].to_bits(){stress_bad+=1;if stress_bad<8 {println!("STRESS_MISMATCH index={i} channel={c} code={code:?} actual={actual:.17e} CPU={:.17e} parts={:?}",expected[c],[stress_gpu[3*i][c],stress_gpu[3*i+1][c],stress_gpu[3*i+2][c]]);}}}}
 println!("SOURCE_CONVERSION_STRESS all_65536_neutral_codes_plus_65536_seeded_colours f64_bit_mismatch_channels={stress_bad} legacy_f32_source_bit_mismatch_channels={stress_high_bad} maximum_absolute_error={stress_max:.9e}");
 let mut accepted=0;let mut rejected=0;let mut failing_cases=0;let mut new_failures=0;let mut old_failures=0;let mut flags_failures=0;
 for beta in [1.,2.5,4.] {for gamma in [0.1,0.65,3.] {for k in [0.,0.02] {for ev in [-32.,3.,16.,32.] {
  let roll=RollSettings{gamma,..roll};let g=roll.channel_gammas()?;
  if calibrations.iter().any(|cal|(0..3).any(|c|g[c]-k*lr(cal.dmin[c],cal.input_floor)/std::f64::consts::LN_10<=0.)) {rejected+=1;continue;}
  if calibrations.iter().any(|&cal| {let f=value(DVec3::splat(cal.input_floor),cal,g,anchor(cal,g,beta,k),beta,k,ev,DMat3::IDENTITY).abs();let bound=m.x_axis.abs()*f.x+m.y_axis.abs()*f.y+m.z_axis.abs()*f.z;!f.is_finite()||!bound.is_finite()||f.max_element()>f32::MAX as f64/4.||bound.max_element()>f32::MAX as f64/4.}) {rejected+=1;continue;}
  accepted+=1;let uniform=buffer(&device,&params(camera_matrix,g,beta,ev,m),wgpu::BufferUsages::UNIFORM);let reference=buffer(&device,&refs(&calibrations,g,beta,k),wgpu::BufferUsages::STORAGE);let bind=group(&device,&pipe,&input,&output,&uniform,Some(&reference));let gpu=read(&device,&queue,&pipe,&bind,&output,codes.len()as u32,(codes.len()*16)as u64)?;
  let mut failures=[0usize;2];let mut worst=[0_f64;2];let mut indices=[0;2];let mut flags=0;
  for(i,&s)in scans.iter().enumerate(){let cal=calibrations[i];let expected=if beta==1.&&k==0. {m*Inversion::new(cal,roll)?.apply(s)?.rgb*ev.exp2()}else{value(s,cal,g,anchor(cal,g,beta,k),beta,k,ev,m)};let category=usize::from(i>=fur);
   let bits=(0..3).fold(0u32,|a,c|a|(u32::from(s[c]<cal.input_floor)<<c));if gpu[i][3]as u32!=bits{flags+=1;}
   for c in 0..3 {let actual=gpu[i][c]as f64;let ratio=(actual-expected[c]).abs()/(2e-5+2e-4*expected[c].abs());if !actual.is_finite()||ratio>1. {failures[category]+=1;}if ratio>worst[category]{worst[category]=ratio;indices[category]=i;}}
  }
  let count=failures.iter().sum::<usize>();if count+flags>0{failing_cases+=1;}if beta==1.&&k==0. {old_failures+=count;}else{new_failures+=count;}flags_failures+=flags;
  println!("CASE beta={beta} gamma={gamma} k={k} total_EV={ev} failed_channels_fur_boundary={failures:?} worst_tolerance_multiple={worst:?} worst_indices={indices:?} wrong_floor_flags={flags}");
 }}}}
 println!("SUMMARY accepted_cases={accepted} domain/range_rejected_cases={rejected} failing_cases={failing_cases} failing_new_response_channels={new_failures} failing_legacy_channels={old_failures} wrong_floor_flags={flags_failures} unchanged_working_tolerance=2e-5+2e-4*abs(f64_reference)");
 // Timing only: same one-tile source pixels, current output precision, grade and shader shape.
 let n=1024*256;let tile:Vec<_>=(0..n).map(|i|codes[i%fur]).collect();
 let mut pack8=Vec::new();let mut pack48=Vec::new();for _ in 0..5 {let t=Instant::now();std::hint::black_box(code_bytes(&tile));pack8.push(t.elapsed().as_secs_f64()*1000.);let t=Instant::now();std::hint::black_box(packed_ap0(&tile,camera_matrix));pack48.push(t.elapsed().as_secs_f64()*1000.);}
 let data8=code_bytes(&tile);let data48=packed_ap0(&tile,camera_matrix);let src8=buffer(&device,&data8,wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST);let src48=buffer(&device,&data48,wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST);
 let out=device.create_buffer(&wgpu::BufferDescriptor{label:None,size:(n*16)as u64,usage:wgpu::BufferUsages::STORAGE,mapped_at_creation:false});
 let uniform8=buffer(&device,&params(camera_matrix,g,2.5,3.,m),wgpu::BufferUsages::UNIFORM);let reference8=buffer(&device,&refs(&[cal],g,2.5,0.),wgpu::BufferUsages::STORAGE);let bind8=group(&device,&pipe,&src8,&out,&uniform8,Some(&reference8));
 let cached=pipeline(&device,include_str!("../../toe-precision-triple-fma-legacy.wgsl").to_owned(),"main");let a=anchor(cal,g,2.5,0.);let b=split(cal.dmin,cal.input_floor);let oldparams=bytes(&[b[0],g.as_vec3().extend(2.5).to_array(),a.as_vec3().extend(0.).to_array(),m.x_axis.as_vec3().extend(0.).to_array(),m.y_axis.as_vec3().extend(0.).to_array(),m.z_axis.as_vec3().extend(0.).to_array(),[(3.*std::f64::consts::LN_2)as f32,0.,0.,0.],b[1],b[2]]);let uniform48=buffer(&device,&oldparams,wgpu::BufferUsages::UNIFORM);let bind48=group(&device,&cached,&src48,&out,&uniform48,None);
 let gpu8=time_compute(&device,&queue,&pipe,&bind8,n as u32)?;let gpu48=time_compute(&device,&queue,&cached,&bind48,n as u32)?;
 let mut upload8=Vec::new();let mut upload48=Vec::new();for _ in 0..5 {for(buf,data,times)in[(&src8,&data8,&mut upload8),(&src48,&data48,&mut upload48)] {let t=Instant::now();queue.write_buffer(buf,0,data);queue.submit([]);device.poll(wgpu::PollType::wait_indefinitely())?;times.push(t.elapsed().as_secs_f64()*1000.);}}
 println!("BENCHMARK one_tile_pixels={n} source_RGB16_MiB={:.3} source_cached_AP0_triple_MiB={:.3} source_current_AP0_f32_MiB={:.3} working_output_MiB={:.3} target_textures_unchanged=true",data8.len()as f64/1048576.,data48.len()as f64/1048576.,n as f64*16./1048576.,out.size()as f64/1048576.);
 println!("BENCHMARK_CPU_PACK median_ms_RGB16={:.3} median_ms_matrix_and_triple_AP0={:.3} RGB16_runs={pack8:?} cached_AP0_runs={pack48:?}; excludes_LibRaw_decode; RGB16_serialization_not_zero_copy",median(&pack8),median(&pack48));
 println!("BENCHMARK_UPLOAD_QUEUE_DRAIN median_ms_RGB16={:.3} median_ms_cached_AP0={:.3} RGB16_runs={upload8:?} cached_AP0_runs={upload48:?}",median(&upload8),median(&upload48));
 println!("BENCHMARK_COMPUTE_SUBMIT_DRAIN per_dispatch_median_ms_RGB16={:.3} cached_AP0={:.3} RGB16_runs={gpu8:?} cached_AP0_runs={gpu48:?}; five_dispatches_per_batch; no_ACES_rendering; M5_not_M1",median(&gpu8),median(&gpu48));
 if conversion_bad>0||high_bad>0||stress_bad>0||stress_high_bad>0||new_failures>0||flags_failures>0{return Err("packed RGB16 conversion/new-response gate failed; no production upload changes".into());}
 if old_failures>0{return Err("packed RGB16 source/new-response gates pass; expanded legacy extreme-exposure gate still fails separately".into());}Ok(())
})}
