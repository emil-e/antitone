use antitone::{color::{Balance,ap0_to_ap1},processing::{Calibration,Inversion,RollSettings}};
use glam::{DVec3,DMat3};
use std::{error::Error,fs};
use wgpu::util::DeviceExt;
type Result<T>=std::result::Result<T,Box<dyn Error>>;
fn lr(a:f64,b:f64)->f64 {let r=(a-b)/b;if r.abs()<0.5 {r.ln_1p()}else{a.ln()-b.ln()}}
fn la(a:f64)->f64 {if a>0. {a+(-(-a).exp_m1()).ln()}else{(-a.exp_m1()).ln()}}
fn anchors(b:DVec3,w:DVec3,g:DVec3,beta:f64,k:f64)->DVec3 {DVec3::from_array(std::array::from_fn(|c|{let l=lr(b[c],w[c]);beta*l/(g[c]-k*l/std::f64::consts::LN_10)}))}
fn value(s:DVec3,b:DVec3,floor:f64,g:DVec3,a:DVec3,beta:f64,k:f64,ev:f64,m:DMat3)->DVec3 {
 let q=DVec3::from_array(std::array::from_fn(|c| {let l=lr(b[c],s[c].max(floor));let z=beta*l/(g[c]-k*l/std::f64::consts::LN_10);if z==0. {0.} else {z.signum()*((la(z)-la(a[c]))/beta+ev*std::f64::consts::LN_2).exp()}}));m*q
}
fn main()->Result<()> {pollster::block_on(async {
 let cal=Calibration{dmin:DVec3::new(0.16856635015466970,0.13359162596075455,0.11715123642832656),dmax:DVec3::new(0.008061,0.003409,0.001442),input_floor:2.1630933622265155e-5};
 let data=fs::read(".cache/probes/dmin-trace/run2/fur-scan-ap0-f64.bin")?;
 let mut scans:Vec<_>=data.chunks_exact(24).map(|v|DVec3::from_array(std::array::from_fn(|c|f64::from_le_bytes(v[c*8..c*8+8].try_into().unwrap())))).collect();
 let fur=scans.len();
 for eps in [-1e-3,-1e-5,-1e-7,-1e-8,-1e-9,-1e-12,0.,1e-12,1e-9,1e-8,1e-7,1e-5,1e-3] {scans.push(cal.dmin*(1.+eps));for c in 0..3 {let mut s=cal.dmin;s[c]*=1.+eps;scans.push(s);}}
 for v in [cal.dmax,cal.dmax/2.,DVec3::splat(cal.input_floor),DVec3::ZERO,DVec3::new(-0.01,0.02,0.03)] {scans.push(v);}
 let mut bytes=Vec::new();for s in &scans {let hi=s.as_vec3();let low=(*s-hi.as_dvec3()).as_vec3();for row in [hi.extend(0.),low.extend(0.)] {for v in row.to_array(){bytes.extend(v.to_le_bytes());}}}
 let instance=wgpu::Instance::new(wgpu::InstanceDescriptor{backends:wgpu::Backends::METAL,..wgpu::InstanceDescriptor::new_without_display_handle()});let adapter=instance.request_adapter(&Default::default()).await?;
 let(device,queue)=adapter.request_device(&Default::default()).await?;
 let module=device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("isolated toe precision prototype"),source:wgpu::ShaderSource::Wgsl(include_str!("../../toe-precision-split.wgsl").to_owned().into())});
 let pipeline=device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor{label:None,layout:None,module:&module,entry_point:Some("main"),compilation_options:Default::default(),cache:None});
 let input=device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:None,contents:&bytes,usage:wgpu::BufferUsages::STORAGE});
 let output=device.create_buffer(&wgpu::BufferDescriptor{label:None,size:(scans.len()*16) as u64,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
 let read=device.create_buffer(&wgpu::BufferDescriptor{label:None,size:(scans.len()*16) as u64,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
 println!("ISOLATED TWO-PART SOURCE PROTOTYPE ONLY adapter={:?} samples={} fur={} boundary/floor={} unchanged working tolerance=2e-5+2e-4*abs(reference)",adapter.get_info(),scans.len(),fur,scans.len()-fur);
 let m=ap0_to_ap1();let qm=DMat3::from_cols(m.x_axis.as_vec3().as_dvec3(),m.y_axis.as_vec3().as_dvec3(),m.z_axis.as_vec3().as_dvec3());
 let mut failures=0;let mut accepted=0;let mut rejected=0;
 for beta in [1.,2.5,4.] {for gamma in [0.1,0.65,3.] {for k in [0.,0.02] {for ev in [-32.,3.,16.,32.] {
  let roll=RollSettings{gamma,balance:Balance{warmth:-0.30045307385649744,tint:0.013735687288646154},exposure:0.};let g=roll.channel_gammas()?;
  if (0..3).any(|c|g[c]-k*lr(cal.dmin[c],cal.input_floor)/std::f64::consts::LN_10<=0.) {rejected+=1;continue;}
  let a=anchors(cal.dmin,cal.dmax,g,beta,k);let floor_q=value(DVec3::splat(cal.input_floor),cal.dmin,cal.input_floor,g,a,beta,k,ev,DMat3::IDENTITY).abs();let bound=m.x_axis.abs()*floor_q.x+m.y_axis.abs()*floor_q.y+m.z_axis.abs()*floor_q.z;
  if !floor_q.is_finite()||!bound.is_finite()||floor_q.max_element()>f32::MAX as f64/4.||bound.max_element()>f32::MAX as f64/4. {rejected+=1;continue;}
  accepted+=1;
  let params=[cal.dmin.as_vec3().extend(cal.input_floor as f32).to_array(),g.as_vec3().extend(beta as f32).to_array(),a.as_vec3().extend(k as f32).to_array(),m.x_axis.as_vec3().extend(0.).to_array(),m.y_axis.as_vec3().extend(0.).to_array(),m.z_axis.as_vec3().extend(0.).to_array(),[(ev*std::f64::consts::LN_2)as f32,0.,0.,0.],(cal.dmin-cal.dmin.as_vec3().as_dvec3()).as_vec3().extend((cal.input_floor-cal.input_floor as f32 as f64)as f32).to_array()];
  let mut ub=Vec::new();for row in params {for v in row {ub.extend(v.to_le_bytes());}}
  let uniform=device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:None,contents:&ub,usage:wgpu::BufferUsages::UNIFORM});
  let group=device.create_bind_group(&wgpu::BindGroupDescriptor{label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[wgpu::BindGroupEntry{binding:0,resource:input.as_entire_binding()},wgpu::BindGroupEntry{binding:1,resource:output.as_entire_binding()},wgpu::BindGroupEntry{binding:2,resource:uniform.as_entire_binding()}]});
  let mut encoder=device.create_command_encoder(&Default::default());{let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&group,&[]);pass.dispatch_workgroups((scans.len() as u32).div_ceil(64),1,1);}
  encoder.copy_buffer_to_buffer(&output,0,&read,0,read.size());queue.submit([encoder.finish()]);let(tx,rx)=std::sync::mpsc::channel();read.map_async(wgpu::MapMode::Read,..,move|r|tx.send(r).unwrap());device.poll(wgpu::PollType::wait_indefinitely())?;rx.recv()??;
  let mapped=read.get_mapped_range(..)?;let results:Vec<[f32;4]>=mapped.chunks_exact(16).map(|p|std::array::from_fn(|c|f32::from_le_bytes(p[c*4..c*4+4].try_into().unwrap()))).collect();drop(mapped);read.unmap();
  let mut counts=[0;2];let mut worst=[0_f64;2];let mut worst_i=[0;2];let mut quant_fails=[0;2];let mut max_abs=[0_f64;2];let mut flags_bad=0;
  for(i,s)in scans.iter().enumerate(){let expected=if beta==1.&&k==0. {m*Inversion::new(cal,roll)?.apply(*s)?.rgb*ev.exp2()}else{value(*s,cal.dmin,cal.input_floor,g,a,beta,k,ev,m)};
   let qexpected=value(s.as_vec3().as_dvec3(),cal.dmin.as_vec3().as_dvec3(),cal.input_floor as f32 as f64,g.as_vec3().as_dvec3(),a.as_vec3().as_dvec3(),beta as f32 as f64,k as f32 as f64,params[6][0]as f64/std::f64::consts::LN_2,qm);
   let group=usize::from(i>=fur);
   let flags=(0..3).fold(0u32,|n,c|n|(u32::from(s[c]<cal.input_floor)<<c));flags_bad+=usize::from(results[i][3]as u32!=flags);
   for c in 0..3 {let gpu=results[i][c]as f64;let error=(gpu-expected[c]).abs();let ratio=error/(2e-5+2e-4*expected[c].abs());
    if !gpu.is_finite()||ratio>1. {counts[group]+=1;}
    if ratio>worst[group]{worst[group]=ratio;worst_i[group]=i;}max_abs[group]=max_abs[group].max(error);
    if !gpu.is_finite()||(gpu-qexpected[c]).abs()>2e-5+2e-4*qexpected[c].abs(){quant_fails[group]+=1;}
   }
  }
  failures+=counts.iter().sum::<usize>()+flags_bad;
  println!("CASE beta={beta} gamma={gamma} k={k} total_EV={ev} full_f64_failed_channels_fur_boundary={counts:?} worst_tolerance_multiple={worst:?} worst_indices={worst_i:?} max_abs={max_abs:?} quantized_reference_failed_channels={quant_fails:?} wrong_floor_flags={flags_bad}");
  if beta==2.5&&gamma==0.65&&k==0.&&ev==3. {let i=worst_i[1];println!("DEFAULT_BOUNDARY_EXAMPLE index={i} original_scan={:?} uploaded_scan={:?} reference_Dmin={:?} uploaded_Dmin={:?} GPU={:?} CPU={:?}",scans[i],scans[i].as_vec3(),cal.dmin,cal.dmin.as_vec3(),results[i],value(scans[i],cal.dmin,cal.input_floor,g,a,beta,k,ev,m));}
 }}}}
 println!("SUMMARY accepted_cases={accepted} domain/range_rejected_cases={rejected} failing_channels_or_flags={failures}; no production code changed");
 if failures>0{return Err("candidate fails existing f64/GPU gate; investigate input/parameter precision before production integration".into());}Ok(())
})}
