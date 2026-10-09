use antitone::{
    color::{Balance, ap0_to_ap1, AP1_TO_XYZ},
    export::{self, ImageSize, Overwrite},
    processing::{Calibration, Inversion, PhotoSettings, Pipeline, RollSettings},
    raw::{RawFile, Rect}, rendering::{CpuRenderer, OutputTarget}, sampling,
};
use glam::DVec3;
use std::{error::Error, fs, path::Path};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const DIR: &str = ".cache/probes/dmin-trace/run2";
const OLD: DVec3 = DVec3::new(0.140103,0.113156,0.102145);
const DMAX: DVec3 = DVec3::new(0.008061,0.003409,0.001442);
fn roll(ev:f64)->RollSettings {RollSettings{gamma:0.65,balance:Balance{warmth:-0.41,tint:-0.01},exposure:ev}}
fn source(raw:&RawFile,r:Rect)->Result<Vec<DVec3>> {
    let image=raw.camera_image()?;
    let mut out=Vec::with_capacity((r.width*r.height) as usize);
    for y in r.y..r.y+r.height {for x in r.x..r.x+r.width {out.push(image.pixel(x,y)?);}}
    Ok(out)
}
fn stats(p:&[DVec3])->Result<String> {
    let mean=sampling::mean(p)?;
    let sd=(p.iter().fold(DVec3::ZERO,|a,v|a+(*v-mean)*(*v-mean))/p.len() as f64).sqrt();
    let mut y:Vec<_>=p.iter().map(|v|(AP1_TO_XYZ*(*v)).y).collect();y.sort_by(f64::total_cmp);
    Ok(format!("RGB_mean={mean:?} RGB_sd={sd:?} AP1_Y_p05_p50_p95=({:.9},{:.9},{:.9})",y[y.len()/20],y[y.len()/2],y[y.len()*19/20]))
}
fn codes(output:&[glam::Vec3])->Result<Vec<f64>> {
    let xyz=OutputTarget::DisplayP3.rgb_to_xyz();
    output.iter().map(|v|Ok(OutputTarget::DisplayP3.encode((xyz*v.as_dvec3()).y)?)).collect()
}
fn displayed(output:&[glam::Vec3],width:usize)->Result<String> {
    let mut grey=codes(output)?;
    let mean=grey.iter().sum::<f64>()/grey.len() as f64;
    let mut pairs=0;let mut neighbor=0.;
    for i in 0..grey.len() {
        if i%width+1<width {neighbor+=(grey[i]-grey[i+1]).powi(2);pairs+=1;}
        if i+width<grey.len() {neighbor+=(grey[i]-grey[i+width]).powi(2);pairs+=1;}
    }
    let black=output.iter().filter(|v|v.max_element()==0.).count();
    let low=grey.iter().filter(|v|**v<1./255.).count();
    grey.sort_by(f64::total_cmp);
    Ok(format!("n={} exact_black={black} grey_Y_code_below_1of255={low} grey_Y_code_p05_p50_p95=({:.6},{:.6},{:.6}) mean={mean:.6} adjacent_grey_Y_code_RMS={:.6}",output.len(),grey[grey.len()/20],grey[grey.len()/2],grey[grey.len()*19/20],(neighbor/pairs as f64).sqrt()))
}
fn save(name:&str,p:&[DVec3],size:ImageSize)->Result<()> {
    export::tiff(Path::new(&format!("{DIR}/{name}.tif")),size,OutputTarget::DisplayP3,Overwrite::Refuse,|x,y|Ok(p[(y*size.width+x) as usize]))?;Ok(())
}
fn main()->Result<()> {
    fs::create_dir_all(DIR)?;
    let wb=RawFile::open(Path::new("samples/DSC04378.ARW"))?.metadata.cam_mul;
    let mut raw=RawFile::open(Path::new("samples/DSC04393.ARW"))?;
    let matrix=raw.metadata.camera_to_ap0()?;
    let floor=(matrix.x_axis.abs()+matrix.y_axis.abs()+matrix.z_axis.abs()).max_element()/65535.;
    let area=raw.metadata.image_area()?;
    raw.develop_camera(wb)?;
    println!("UNCHANGED CURRENT MODEL. Fixed WB={wb:?}; OLD_Dmin={OLD:?}; Dmax={DMAX:?}; roll={:?}; ASSUMED photo={:?}; input_floor={floor:.12e}. Rounded screenshot values, not exact live grade.",roll(3.),PhotoSettings::default());
    let refs=[
        ("whole-picture-estimate",Rect{x:5588,y:3780,width:32,height:32}),
        ("verified-bottom",Rect{x:5000,y:5660,width:128,height:32}),
        ("left-holdout",Rect{x:36,y:2500,width:32,height:128}),
        ("bottom-right-holdout",Rect{x:7000,y:5644,width:128,height:32}),
    ];
    let mut measured=Vec::new();
    for (name,r) in refs {
        let sensor=raw.region_sensor_diagnostics(r)?;
        if sensor.at_white.iter().chain(&sensor.below_black).any(|n|*n>0) {return Err("reference contains sensor clipping".into());}
        let camera=source(&raw,r)?;
        let cm=sampling::mean(&camera)?;
        let cs=(camera.iter().fold(DVec3::ZERO,|a,v|a+(*v-cm)*(*v-cm))/camera.len() as f64).sqrt();
        let b=matrix*cm;
        Calibration{dmin:b,dmax:DMAX,input_floor:floor}.validate()?;
        println!("REFERENCE name={name} {r:?} n={} camera_mean={cm:?} camera_CV={:?} AP0_mean={b:?} ratio_to_old={:?} sensor={sensor:?}",camera.len(),cs/cm,b/OLD);
        measured.push(b);
    }
    let base=measured[0];
    let cases=[("screenshot-dmin",OLD),("clear-margin-dmin",measured[1]),("whole-picture-dmin",base)];
    // Reopenable full-resolution view of the strongest coherent interior patch.
    let r=Rect{x:5396,y:3620,width:416,height:352};
    let inspect:Vec<_>=source(&raw,r)?.iter().map(|c|ap0_to_ap1()*matrix*(*c)*4.).collect();
    save("whole-picture-base-source-inspection-plus2ev",&inspect,ImageSize{width:r.width,height:r.height})?;
    let renderer=CpuRenderer::new(OutputTarget::DisplayP3)?;
    let regions=[
        ("forehead",Rect{x:5750,y:2400,width:512,height:384}),
        ("muzzle",Rect{x:5100,y:3350,width:512,height:384}),
        ("chest",Rect{x:6650,y:4200,width:512,height:384}),
        ("wood",Rect{x:4700,y:4200,width:128,height:128}),
        ("sky",Rect{x:1800,y:400,width:512,height:384}),
    ];
    let mut csv=String::from("roi,case,x,y,camera_r,camera_g,camera_b,scan_ap0_r,scan_ap0_g,scan_ap0_b,inv_ap0_r,inv_ap0_g,inv_ap0_b,working_ap1_r,working_ap1_g,working_ap1_b,range_r,range_g,range_b,target_linear_r,target_linear_g,target_linear_b,grey_Y_code\n");
    let mut gpu_source=Vec::new();
    for (name,r) in regions {
        let sensor=raw.region_sensor_diagnostics(r)?;
        let camera=source(&raw,r)?;
        let scans:Vec<_>=camera.iter().map(|c|matrix*(*c)).collect();
        let cm=sampling::mean(&camera)?;
        let cs=(camera.iter().fold(DVec3::ZERO,|a,v|a+(*v-cm)*(*v-cm))/camera.len() as f64).sqrt();
        println!("ROI_SOURCE name={name} {r:?} n={} camera_mean={cm:?} camera_sd={cs:?} scan_AP0_mean={:?} sensor={sensor:?}",camera.len(),sampling::mean(&scans)?);
        let mut baseline=Vec::new();
        for (case,b) in cases {
            let cal=Calibration{dmin:b,dmax:DMAX,input_floor:floor};
            let inv=Inversion::new(cal,roll(3.))?;
            assert_eq!(inv.apply(b)?.rgb,DVec3::ZERO);
            let pipe=Pipeline::new(cal,roll(3.),PhotoSettings::default())?;
            let inversions=scans.iter().map(|s|inv.apply(*s)).collect::<std::result::Result<Vec<_>,_>>()?;
            let inv_rgb:Vec<_>=inversions.iter().map(|v|v.rgb).collect();
            let pre:Vec<_>=inv_rgb.iter().map(|q|ap0_to_ap1()*(*q)).collect();
            let working=scans.iter().map(|s|Ok(pipe.apply(*s)?.rgb)).collect::<Result<Vec<_>>>()?;
            let range:Vec<_>=working.iter().map(|w|w.clamp(DVec3::ZERO,DVec3::splat(1024.))).collect();
            let output=renderer.render(&working)?;
            let explicit_range=renderer.render(&range)?;
            assert_eq!(output,explicit_range);
            let code=codes(&output)?;
            let ap0_negative=inv_rgb.iter().filter(|q|q.min_element()<0.).count();
            let ap1_negative=working.iter().filter(|q|q.min_element()<0.).count();
            let all_negative=working.iter().filter(|q|q.max_element()<=0.).count();
            let positive_black=working.iter().zip(&output).filter(|(q,o)|q.max_element()>0.&&o.max_element()==0.).count();
            let floored=inversions.iter().filter(|q|q.floored.into_iter().any(|v|v)).count();
            let cam_base=matrix.inverse()*b;
            let cam_exceeds=camera.iter().filter(|c|(**c-cam_base).max_element()>1e-12).count();
            let ap0_exceeds=scans.iter().filter(|s|(**s-b).max_element()>1e-12).count();
            let matrix_only=camera.iter().zip(&scans).filter(|(c,s)|(**c-cam_base).max_element()<=1e-12&&(**s-b).max_element()>1e-12).count();
            println!("ROI_STAGE name={name} case={case} floored_inputs={floored} camera_any_above_reference={cam_exceeds} AP0_any_above_reference={ap0_exceeds} AP0_only_overshoot_with_camera_bounded={matrix_only} inverse_AP0_any_negative={ap0_negative} post_AP1_any_negative={ap1_negative} post_AP1_all_nonpositive={all_negative} rendered_black_despite_positive_AP1={positive_black} explicit_range_vs_production_exact=true");
            println!("ROI_WORKING name={name} case={case} pre_exposure_{}",stats(&pre)?);
            println!("ROI_DISPLAY name={name} case={case} {}",displayed(&output,r.width as usize)?);
            if case=="screenshot-dmin" {baseline=output.clone();}
            else {
                let regained=baseline.iter().zip(&output).filter(|(a,b)|a.max_element()==0.&&b.max_element()>0.).count();
                let visible_regained=baseline.iter().zip(&code).filter(|(a,b)|a.max_element()==0.&&**b>=1./255.).count();
                let retained_black=baseline.iter().zip(&output).filter(|(a,b)|a.max_element()==0.&&b.max_element()==0.).count();
                println!("ROI_CHANGE name={name} case={case} old_black_now_nonblack={regained} old_black_now_grey_Y_code_ge_1of255={visible_regained} still_black={retained_black}");
            }
            if name!="wood" {save(&format!("{name}-{case}-unchanged-grade"),&working,ImageSize{width:r.width,height:r.height})?;}
            let mut examples=Vec::new();
            for category in 0..3 {
                let mut count=0;
                for i in 0..working.len() {
                    let matches=match category {
                        0=>output[i].max_element()==0.,
                        1=>output[i].max_element()>0.&&code[i]<1./255.,
                        _=>baseline[i].max_element()==0.&&code[i]>=1./255.,
                    };
                    if matches {examples.push(i);count+=1;if count==6 {break;}}
                }
            }
            for i in examples {
                let c=camera[i];let s=scans[i];let q=inv_rgb[i];let w=working[i];let z=range[i];let o=output[i];
                csv.push_str(&format!("{name},{case},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",r.x+i as u32%r.width,r.y+i as u32/r.width,c.x,c.y,c.z,s.x,s.y,s.z,q.x,q.y,q.z,w.x,w.y,w.z,z.x,z.y,z.z,o.x,o.y,o.z,code[i]));
            }
            if case=="whole-picture-dmin" && name!="sky" && name!="wood" {
                let exp:Vec<_>=working.iter().map(|w|*w*4.).collect();
                println!("ROI_DISPLAY_EXTRA_EXPOSURE name={name} whole_picture_Dmin_plus2EV {}",displayed(&renderer.render(&exp)?,r.width as usize)?);
                save(&format!("{name}-whole-picture-dmin-plus2EV"),&exp,ImageSize{width:r.width,height:r.height})?;
            }
        }
        if name=="forehead"||name=="muzzle"||name=="chest" {
            for y in 0..32 {for x in 0..32 {
                gpu_source.push(scans[((2*y+1)*r.height/64*r.width+(2*x+1)*r.width/64) as usize]);
            }}
        }
    }
    let crop=Rect{x:192,y:96,width:8136,height:5424};
    assert!(crop.x>=area.x&&crop.y>=area.y&&crop.x+crop.width<=area.x+area.width&&crop.y+crop.height<=area.y+area.height);
    let size=ImageSize{width:900,height:600};
    let mut scans=Vec::new();
    for y in 0..size.height {for x in 0..size.width {
        scans.push(matrix*raw.camera_image()?.pixel(crop.x+(2*x+1)*crop.width/(2*size.width),crop.y+(2*y+1)*crop.height/(2*size.height))?);
    }}
    for (case,b,ev) in [("screenshot-dmin",OLD,3.),("clear-margin-dmin",measured[1],3.),("whole-picture-dmin",base,3.),("whole-picture-dmin-plus2EV",base,5.)] {
        let pipe=Pipeline::new(Calibration{dmin:b,dmax:DMAX,input_floor:floor},roll(ev),PhotoSettings::default())?;
        let working=scans.iter().map(|s|Ok(pipe.apply(*s)?.rgb)).collect::<Result<Vec<_>>>()?;
        println!("FRAME case={case} ev={ev} {}",displayed(&renderer.render(&working)?,size.width as usize)?);
        save(&format!("dog-{case}"),&working,size)?;
    }
    fs::write(format!("{DIR}/representative-stage-traces.csv"),csv)?;
    let mut bytes=Vec::new();for s in gpu_source {for value in s.to_array() {bytes.extend(value.to_le_bytes());}}
    fs::write(format!("{DIR}/fur-scan-ap0-f64.bin"),bytes)?;
    fs::write(format!("{DIR}/whole-dmin.txt"),format!("{:.17} {:.17} {:.17} {:.17}\n",base.x,base.y,base.z,floor))?;
    Ok(())
}
