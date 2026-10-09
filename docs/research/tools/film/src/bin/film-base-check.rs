use antitone::{color::{Balance,ap0_to_ap1},processing::{Calibration,Inversion,RollSettings},raw::Rect,scan::Scan,rendering::{CpuRenderer,OutputTarget}};
use glam::DVec3;
use std::{error::Error,path::Path};
fn toe(s:DVec3,cal:Calibration,g:DVec3)->DVec3 {
    let mut q=DVec3::ZERO;
    for c in 0..3 {
        let a=2.5*((cal.dmin[c]-s[c])/s[c]).ln_1p()/g[c];
        let denominator=(2.5*(cal.dmin[c]/cal.dmax[c]).ln()/g[c]).exp_m1();
        q[c]=a.signum()*(a.exp_m1().abs()/denominator).powf(0.4);
    }
    q
}
fn main()->Result<(),Box<dyn Error>> {
    let current=RollSettings{gamma:0.65,balance:Balance{warmth:-0.29553872209057486,tint:0.014664899967910436},exposure:0.};
    let fitted=RollSettings{gamma:0.65,balance:Balance{warmth:-0.2925437483636636,tint:0.010488703357928909},exposure:0.};
    let evs:[f64;3]=[2.420983,2.745805,2.803771];
    for (photo,id) in [4393,4399,4415].iter().enumerate() {
        let scan=Scan::open(Path::new(&format!("samples/DSC{id:05}.ARW")),Some([2133.,1024.,1771.,1024.]))?;
        let cal=Calibration{dmin:DVec3::new(0.140103,0.113156,0.102145),dmax:DVec3::new(0.008061,0.003409,0.001442),input_floor:scan.input_floor};
        let inverse=scan.camera_to_ap0.inverse();let camera_base=inverse*cal.dmin;
        let old=Inversion::new(cal,current)?;let basis=ap0_to_ap1();
        let mut sources=Vec::new();let mut old_values=Vec::new();let mut new_values=Vec::new();let mut held_values=Vec::new();
        let mut ap0_above=0;let mut cam_above=0;let mut ap0_only=0;
        let area=Rect{x:192,y:96,width:8136,height:5424};
        for y in 0..600 {for x in 0..900 {
            let s=scan.pixel(area.x+(2*x+1)*area.width/1800,area.y+(2*y+1)*area.height/1200)?;
            let camera=inverse*s;let ap0_exceeds=(s-cal.dmin).max_element()>1e-12;
            let cam_exceeds=(camera-camera_base).max_element()>1e-12;
            ap0_above+=usize::from(ap0_exceeds);cam_above+=usize::from(cam_exceeds);ap0_only+=usize::from(ap0_exceeds&&!cam_exceeds);
            old_values.push(basis*old.apply(s)?.rgb*8.);
            new_values.push(basis*toe(s,cal,fitted.channel_gammas()?)*evs[photo].exp2());
            held_values.push(basis*toe(s,cal,current.channel_gammas()?)*evs[photo].exp2());
            sources.push((s,camera,cam_exceeds));
        }}
        let renderer=CpuRenderer::new(OutputTarget::DisplayP3)?;
        let old_rgb=renderer.render(&old_values)?;let new_rgb=renderer.render(&new_values)?;let held_rgb=renderer.render(&held_values)?;
        let mut old_black=0;let mut new_black=0;let mut gained=0;let mut lost=0;let mut gained_camera_bounded=0;let mut old_negative=0;let mut new_negative=0;let mut held_gained=0;let mut old_gained_max=0_f32;
        for i in 0..sources.len() {
            let was=old_rgb[i].max_element()==0.;let now=new_rgb[i].max_element()==0.;
            old_black+=usize::from(was);new_black+=usize::from(now);old_negative+=usize::from(old_values[i].max_element()<=0.);new_negative+=usize::from(new_values[i].max_element()<=0.);
            if !was && held_rgb[i].max_element()==0. {held_gained+=1;}
            if was && !now {lost+=1;}
            if !was && now {
                gained+=1;gained_camera_bounded+=usize::from(!sources[i].2);old_gained_max=old_gained_max.max(old_rgb[i].max_element());
                if gained<=3 {println!("EXAMPLE photo={id} index={i} AP0_scan/base={:?} camera_scan/base={:?} old_AP1={:?} new_AP1={:?}",sources[i].0/cal.dmin,sources[i].1/camera_base,old_values[i],new_values[i]);}
            }
        }
        println!("PHOTO {id}: n={} AP0_any_above_base={ap0_above} camera_any_above_base={cam_above} AP0_above_with_all_camera_bounded={ap0_only}; old_black={old_black} new_black={new_black} newly_black={gained} newly_visible={lost} newly_black_all_camera_bounded={gained_camera_bounded}; all_nonpositive_AP1 old={old_negative} new={new_negative}; toe_with_current_balance_newly_black={held_gained}; maximum_previous_display_RGB_among_new_black={old_gained_max:.9e}",sources.len());
        if photo==0 {
            println!("CAMERA_TO_AP0={:?}; camera_base={camera_base:?}",scan.camera_to_ap0);
            for factors in [DVec3::new(1.,0.8,1.),DVec3::new(0.8,1.,1.),DVec3::new(1.,1.,0.8)] {
                let s=scan.camera_to_ap0*(camera_base*factors);
                println!("SYNTHETIC_CAMERA_ATTENUATION factors={factors:?} AP0_scan/base={:?}",s/cal.dmin);
            }
        }
    }
    Ok(())
}
