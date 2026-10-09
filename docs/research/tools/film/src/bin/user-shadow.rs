use antitone::{color::{Balance,ap0_to_ap1},processing::{Calibration,Inversion,PhotoSettings,Pipeline,RollSettings},raw::RawFile,scan::Scan,rendering::{CpuRenderer,OutputTarget}};
use glam::DVec3;
fn main() {
    let path=std::path::Path::new("samples/DSC04393.ARW");
    let first=RawFile::open(std::path::Path::new("samples/DSC04378.ARW")).unwrap();
    let meta=RawFile::open(path).unwrap().metadata;
    println!("Identified screenshot by embedded-preview geometry: DSC04393.ARW. Rounded visible settings only, photo settings NOT supplied.");
    println!("assumed first-import WB={:?}; selected-photo WB={:?}",first.metadata.cam_mul,meta.cam_mul);
    let mut scan=Scan::open(path,Some(first.metadata.cam_mul)).unwrap();
    let local=std::env::args().any(|v|v=="--local-dmin");
    let unanchored=std::env::args().any(|v|v=="--no-subtraction");
    let toe=std::env::args().any(|v|v=="--toe-example");
    // Controlled diagnostic counterfactual only; no editor state or saved reference changes.
    let dmin=if local {scan.reference(antitone::raw::Rect{x:4000,y:5680,width:128,height:32}).unwrap()} else {DVec3::new(0.140103,0.113156,0.102145)};
    let cal=Calibration{dmin,dmax:DVec3::new(0.008061,0.003409,0.001442),input_floor:scan.input_floor};
    let roll=RollSettings{gamma:0.65,balance:Balance{warmth:-0.41,tint:-0.01},exposure:3.};
    let photo=PhotoSettings::default();
    for (name,x,y,w,h) in [("bottom_center_known_patch",4000,5644,128,32),("bottom_center_lower",4000,5680,128,32),("left_margin_mid",36,2500,32,128),("right_margin_mid",8520,2500,32,128),("bottom_left",1000,5644,128,32),("bottom_right",7000,5644,128,32)] {
        let area=antitone::raw::Rect{x,y,width:w,height:h};
        println!("FRAME_MARGIN {name} {area:?} reference={:?}",scan.reference(area));
    }
    println!("cal={cal:?} roll={roll:?} ASSUMED photo={photo:?} area={:?}",scan.area);
    let inv=Inversion::new(cal,roll).unwrap();let pipe=Pipeline::new(cal,roll,photo).unwrap();let basis=ap0_to_ap1();let renderer=CpuRenderer::new(OutputTarget::DisplayP3).unwrap();
    let gammas=roll.channel_gammas().unwrap();
    let film=|s:DVec3| if unanchored {
        DVec3::from_array(std::array::from_fn(|c| (cal.dmax[c]/s[c].max(cal.input_floor)).powf(1./gammas[c])))
    } else if toe {
        // Illustrative beta=2 characteristic family, NOT a fit to the user's film.
        // delta = (gamma/beta)*log10(1+K*E^beta), with unchanged asymptotic gamma.
        DVec3::from_array(std::array::from_fn(|c| {
            let beta=2.;let s=s[c].max(cal.input_floor);
            let n=(beta/gammas[c]*(cal.dmin[c]/s).ln()).exp_m1();
            let d=(beta/gammas[c]*(cal.dmin[c]/cal.dmax[c]).ln()).exp_m1();
            let v=n/d;v.signum()*v.abs().powf(1./beta)
        }))
    } else {inv.apply(s).unwrap().rgb};
    println!("MODEL no_subtraction={unanchored} illustrative_toe_beta2={toe} channel_gammas={gammas:?}");
    for (name,s) in [("Dmin",cal.dmin),("Dmax",cal.dmax)] {
        let a=film(s);let b=basis*a*8.;let output=renderer.render(&[b]).unwrap()[0];
        println!("ANCHOR={name} inverted_AP0={a:?} working_AP1={b:?} rendered={output:?} encoded={:?}",output.to_array().map(|v|OutputTarget::DisplayP3.encode(v as f64).unwrap()));
    }
    let regions=[("whole_frame",scan.area.x,scan.area.y,scan.area.width,scan.area.height), ("dog_forehead",5750,2400,700,500),("dog_muzzle",5100,3350,900,600),("dark_chest",6650,4200,700,1000),("light_fur",6450,3950,400,650),("wood_rail",4600,4200,350,400),("sky",1300,700,600,400)];
    for (name,x0,y0,w,h) in regions {
        let mut scan_min=DVec3::splat(f64::INFINITY);let mut scan_max=DVec3::splat(f64::NEG_INFINITY);let mut ap1_min=scan_min;let mut ap1_max=scan_max;
        let mut above=[0usize;3];let mut ap0negative=0;let mut ap1negative=0;let mut allnonpos=0;let mut floors=0;let mut working=Vec::new();let mut ys=Vec::new();
        for j in 0..64 {for i in 0..64 {
            let s=scan.pixel(x0+(2*i+1)*w/128,y0+(2*j+1)*h/128).unwrap();let mut p=pipe.apply(s).unwrap();let a=film(s);let b=basis*a;
            if unanchored || toe {p.rgb=b*8.;}
            scan_min=scan_min.min(s);scan_max=scan_max.max(s);ap1_min=ap1_min.min(p.rgb);ap1_max=ap1_max.max(p.rgb);
            for c in 0..3 {above[c]+=usize::from(s[c]>cal.dmin[c]);}
            ap0negative+=usize::from(a.min_element()<0.);ap1negative+=usize::from(b.min_element()<0.);allnonpos+=usize::from(p.rgb.max_element()<=0.);floors+=usize::from(p.floored.into_iter().any(|v|v));
            ys.push((antitone::color::AP1_TO_XYZ*p.rgb).y);working.push(p.rgb);
        }}
        let output=renderer.render(&working).unwrap();let mut black=0;let mut sub1=0;let mut encoded:Vec<f64>=Vec::new();
        for p in output {black+=usize::from(p.max_element()==0.);let e=OutputTarget::DisplayP3.encode(p.max_element() as f64).unwrap();sub1+=usize::from(e<1./255.);encoded.push(e);}
        encoded.sort_by(f64::total_cmp);ys.sort_by(f64::total_cmp);
        println!("ROI={name} n={} scan_min={scan_min:?} scan_max={scan_max:?} scan_above_Dmin={above:?} negative_inverted_AP0={ap0negative} negative_prephoto_AP1={ap1negative} all_nonpositive_working={allnonpos} floors={floors} rendered_black={black} max_channel_below_1of255={sub1} working_Y_p05_p50_p95=({:.9},{:.9},{:.9}) encoded_max_p05_p50_p95=({:.9},{:.9},{:.9})",working.len(),ys[204],ys[2048],ys[3891],encoded[204],encoded[2048],encoded[3891]);
    }
    let width=768;let height=512;let mut input=Vec::new();
    for y in 0..height {for x in 0..width {let s=scan.pixel(scan.area.x+(2*x+1)*scan.area.width/(2*width),scan.area.y+(2*y+1)*scan.area.height/(2*height)).unwrap();input.push(if unanchored || toe {basis*film(s)*8.} else {pipe.apply(s).unwrap().rgb});}}
    // Static diagnostic only, with the app's actual TIFF renderer/profile, not an interactive viewer.
    antitone::export::tiff(std::path::Path::new(if toe {".cache/probes/black-user-toe-example.tif"} else if unanchored {".cache/probes/black-user-no-subtraction.tif"} else if local {".cache/probes/black-user-local-dmin.tif"} else {".cache/probes/black-user-assumed-default-photo-margins.tif"}),antitone::export::ImageSize{width,height},OutputTarget::DisplayP3,antitone::export::Overwrite::Refuse,|x,y|Ok(input[(y*width+x) as usize])).unwrap();
}
