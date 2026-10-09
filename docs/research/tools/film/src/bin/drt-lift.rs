use antitone::{
    color::{Balance, AP1_TO_XYZ},
    export::{self, ImageSize, Overwrite},
    processing::{Calibration, PhotoSettings, Pipeline, RollSettings},
    raw::{RawFile, Rect},
    rendering::{CpuRenderer, OutputTarget},
    scan::Scan,
};
use glam::DVec3;
use std::{error::Error, fs, path::Path, process::Command};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const DIR: &str = ".cache/probes/drt-lift/run3";
const WIDTH: u32 = 720;
const HEIGHT: u32 = 480;

fn luminance(rgb: DVec3) -> f64 { (AP1_TO_XYZ * rgb).y }
fn render_y(renderer: &CpuRenderer, rgb: &[DVec3], ev: f64) -> Result<f64> {
    let scaled: Vec<_> = rgb.iter().map(|v| *v * ev.exp2()).collect();
    let output = renderer.render(&scaled)?;
    let matrix = OutputTarget::DisplayP3.rgb_to_xyz();
    Ok(output.iter().map(|v| (matrix * v.as_dvec3()).y).sum::<f64>() / rgb.len() as f64)
}
fn decode(value: f64) -> f64 {
    value.signum() * if value.abs() <= 0.04045 { value.abs()/12.92 } else { ((value.abs()+0.055)/1.055).powf(2.4) }
}
fn native_inverse(name: &str, values: &[f64], ceiling: f64) -> Result<Vec<f64>> {
    let invalid = values.iter().filter(|v| !v.is_finite() || **v < 0. || **v > ceiling).count();
    if invalid != 0 { return Err(format!("{name}: {invalid}/{} requests outside declared inverse domain 0..{ceiling}",values.len()).into()); }
    let input = format!("{DIR}/{name}-inverse-input.f32");
    let output = format!("{DIR}/{name}-inverse-output.f32");
    fs::write(&input,values.iter().flat_map(|v| (*v as f32).to_le_bytes()).collect::<Vec<_>>())?;
    let status = Command::new(".cache/probes/drt-lift-neutral").args([&input,&output]).status()?;
    if !status.success() { return Err(format!("{name}: native inverse failed").into()); }
    let bytes = fs::read(output)?;
    if bytes.len() != values.len()*4 { return Err("inverse length mismatch".into()); }
    Ok(bytes.chunks_exact(4).map(|p| f32::from_le_bytes(p.try_into().unwrap()) as f64).collect())
}
fn density(scan: DVec3, cal: Calibration) -> f64 {
    // Effective AP0 RGB log ratios, NOT calibrated spectral optical densities.
    let normalized = (cal.dmin / scan.max(DVec3::splat(cal.input_floor))).ln() / (cal.dmin / cal.dmax).ln();
    normalized.element_sum()/3.
}
fn candidate(name: &str, base: &[DVec3], densities: &[f64], white: f64, white_code: f64, ceiling: f64) -> Result<Vec<DVec3>> {
    if name == "current" {return Ok(base.to_vec());}
    if name == "lift-black" || name == "lift-radius-black" {
        // Independent, explicit baseline experiment, NOT an inferred film calibration.
        // Neutral offset with white retained: changing zero placement is not toe compensation.
        let amount=0.001;
        let shifted: Vec<_>=base.iter().map(|v| (*v+DVec3::splat(amount))/(1.+amount)).collect();
        println!("illustrative neutral black placement=+{amount}; Dmin no longer forced to scene zero");
        return candidate(if name=="lift-radius-black" {"lift-radius-toe"} else {"lift-linear"},&shifted,densities,white,white_code,ceiling);
    }
    if name == "lift-channel" {
        let values: Vec<_> = base.iter().flat_map(|v| v.to_array().map(|c| white*c.abs())).collect();
        let inverse = native_inverse(name,&values,ceiling)?;
        return Ok(base.iter().zip(inverse.chunks_exact(3)).map(|(v,e)| DVec3::new(v.x.signum()*e[0],v.y.signum()*e[1],v.z.signum()*e[2])).collect());
    }
    let radius = name.starts_with("lift-radius");
    let toe = name == "lift-toe" || name == "lift-radius-toe";
    let ys: Vec<_> = base.iter().map(|v| if radius {luminance(*v * *v).sqrt()} else {luminance(v.max(DVec3::ZERO))}).collect();
    let desired: Vec<_> = ys.iter().zip(densities).map(|(&y,&d)| {
        if name == "lift-density" {decode(white_code*d.max(0.))}
        else if toe && y >= 0.18 {0.}
        else {white*y}
    }).collect();
    let inverse = native_inverse(name,&desired,ceiling)?;
    let mut density_zero_on_positive = 0;
    let result = base.iter().zip(ys).zip(inverse).map(|((&v,y),e)| {
        if y == 0. {return v;}
        if toe && y >= 0.18 {return v;}
        if name == "lift-density" && e == 0. {density_zero_on_positive += 1;}
        let ratio = e/y;
        if toe {
            // Deliberate partial compensation: smooth log-gain fade from .1 to .18.
            let t = ((y-0.1)/0.08).clamp(0.,1.);
            let weight = 1.-t*t*(3.-2.*t);
            v * (ratio.ln()*weight).exp()
        } else {v*ratio}
    }).collect();
    println!("candidate={name} density_reference_zero_on_positive_working={density_zero_on_positive}");
    Ok(result)
}
fn exposure_match(renderer: &CpuRenderer, pixels: &[DVec3], goal: f64) -> Result<f64> {
    let mut lo: f64 = -16.;let mut hi: f64 = 16.;
    if !(render_y(renderer,pixels,lo)?..=render_y(renderer,pixels,hi)?).contains(&goal) {return Err("exposure-match goal not bracketed".into());}
    for _ in 0..24 {let mid=(lo+hi)*0.5;if render_y(renderer,pixels,mid)? < goal {lo=mid;} else {hi=mid;}}
    Ok((lo+hi)*0.5)
}
fn save(path: &str, pixels: &[DVec3], ev: f64, target: OutputTarget) -> Result<()> {
    export::tiff(Path::new(path),ImageSize{width:WIDTH,height:HEIGHT},target,Overwrite::Refuse,|x,y| Ok(pixels[(y*WIDTH+x) as usize]*ev.exp2()))?;
    Ok(())
}
fn main() -> Result<()> {
    fs::create_dir_all(DIR)?;
    let renderer = CpuRenderer::new(OutputTarget::DisplayP3)?;
    let neutral = CpuRenderer::new(OutputTarget::Srgb)?;
    let white = neutral.render(&[DVec3::ONE])?[0].as_dvec3().element_sum()/3.;
    let white_code = OutputTarget::Srgb.encode(white)?;
    let ceiling = neutral.render(&[DVec3::splat(64.)])?[0].as_dvec3().element_sum()/3.;
    println!("ISOLATED OFFLINE EXPLORATION. Unmodified actual ACES forward/public inverse. No editor changes; no LUT replacement for ACES. Fixed 100nit neutral reference independent of selected target.");
    println!("white_matched_reference scene1_display={white:.12} encoded_white={white_code:.12} inverse_study_ceiling={ceiling:.12}");
    let ramp: Vec<_> = (0..=256).map(|i| DVec3::splat((i as f64/256.).powi(4))).collect();
    let zeros=vec![0.;ramp.len()];
    let lifted=candidate("lift-linear",&ramp,&zeros,white,white_code,ceiling)?;
    let mut max_identity: f64=0.;let mut max_p3: f64=0.;
    let expected: Vec<_>=ramp.iter().map(|v|v.x*white).collect();
    let mut ramp_csv=String::from("reference,desired_display,lifted_scene,ev,current_display,lifted_display\n");
    for ev in [-2_f64,-1.,0.,1.,2.] {
        let a=neutral.render(&ramp.iter().map(|v|*v*ev.exp2()).collect::<Vec<_>>())?;
        let b=neutral.render(&lifted.iter().map(|v|*v*ev.exp2()).collect::<Vec<_>>())?;
        let p=renderer.render(&lifted.iter().map(|v|*v*ev.exp2()).collect::<Vec<_>>())?;
        for i in 0..ramp.len() {
            let y=b[i].as_dvec3().element_sum()/3.;
            if ev==0. {max_identity=max_identity.max((y-expected[i]).abs());}
            max_p3=max_p3.max((p[i].as_dvec3().element_sum()/3.-y).abs());
            ramp_csv += &format!("{},{},{},{},{},{}\n",ramp[i].x,expected[i],lifted[i].x,ev,a[i].as_dvec3().element_sum()/3.,y);
        }
    }
    fs::write(format!("{DIR}/ramps.csv"),ramp_csv)?;
    println!("neutral_ramp_identity_max_error={max_identity:.9e} neutral_sRGB_P3_max_error={max_p3:.9e}");
    // Colour stress: tone-only lift does not constitute a full colour inverse.
    let mut colors=Vec::new();
    for level in [0.01,0.05,0.18,0.5] {for ray in [DVec3::ONE,DVec3::new(1.,0.1,0.1),DVec3::new(0.1,1.,0.1),DVec3::new(0.1,0.1,1.),DVec3::new(1.,1.,0.1),DVec3::new(0.1,1.,1.),DVec3::new(1.,0.1,1.)] {colors.push(ray*level);}}
    // Signed-domain stress: shared positive-luminance division can amplify a
    // mostly-negative vector arbitrarily. A radial carrier is continuous there.
    let stress=vec![DVec3::new(-0.04,1e-12,0.),DVec3::new(-0.04,1e-20,0.),DVec3::splat(-0.001),DVec3::ZERO,DVec3::splat(0.18),DVec3::splat(1.),DVec3::splat(8.),DVec3::splat(1024.),DVec3::splat(65536.)];
    let stress_zero=vec![0.;stress.len()];
    let safe=candidate("lift-radius-toe",&stress,&stress_zero,white,white_code,ceiling)?;
    let unsafe_first=candidate("lift-linear",&stress[..2],&stress_zero[..2],white,white_code,ceiling)?;
    for (index,(input,output)) in stress.iter().zip(&safe).enumerate() {
        println!("SIGNED_HDR input={input:?} radius_toe={output:?} original_luma_lift={:?}",unsafe_first.get(index));
        assert!(output.is_finite());
        if luminance(*input * *input).sqrt()>=0.18 {assert_eq!(*input,*output);}
    }
    assert!(candidate("lift-linear",&stress[6..],&stress_zero[6..],white,white_code,ceiling).is_err());
    let dense:Vec<_>=(0..=65536).map(|i|DVec3::splat(i as f64/65536.)).collect();
    let dense_zero=vec![0.;dense.len()];
    let shaped=candidate("lift-radius-toe",&dense,&dense_zero,white,white_code,ceiling)?;
    assert!(shaped.windows(2).all(|v|v[0].x<=v[1].x));
    let mut direction_error=0_f64;
    let radius_colors=candidate("lift-radius-toe",&colors,&vec![0.;colors.len()],white,white_code,ceiling)?;
    for (a,b) in colors.iter().zip(&radius_colors) {direction_error=direction_error.max((a.normalize()-b.normalize()).length());}
    println!("radius_toe_monotonic_65537_nodes=true max_working_RGB_direction_error={direction_error:.9e} HDR_above_pivot_exactly_unchanged=true");
    let color_zero=vec![0.;colors.len()];
    let scalar=candidate("lift-linear",&colors,&color_zero,white,white_code,ceiling)?;
    let channel=candidate("lift-channel",&colors,&color_zero,white,white_code,ceiling)?;
    let native_scalar=renderer.render(&scalar)?;let native_channel=renderer.render(&channel)?;
    let mut csv=String::from("r,g,b,scalar_r,scalar_g,scalar_b,channel_r,channel_g,channel_b,xy_change_from_scalar\n");
    let xyz=OutputTarget::DisplayP3.rgb_to_xyz();
    for i in 0..colors.len() {
        let a=xyz*native_scalar[i].as_dvec3();let b=xyz*native_channel[i].as_dvec3();
        let delta=((a.truncate()/a.element_sum())-(b.truncate()/b.element_sum())).length();
        csv+=&format!("{},{},{},{},{},{},{},{},{},{}\n",colors[i].x,colors[i].y,colors[i].z,native_scalar[i].x,native_scalar[i].y,native_scalar[i].z,native_channel[i].x,native_channel[i].y,native_channel[i].z,delta);
    }
    fs::write(format!("{DIR}/color-stress.csv"),csv)?;
    let wb=RawFile::open(Path::new("samples/DSC04378.ARW"))?.metadata.cam_mul;
    for id in [4393,4399] {
        let path=format!("samples/DSC{id:05}.ARW");let scan=Scan::open(Path::new(&path),Some(wb))?;
        let cal=Calibration{dmin:DVec3::new(0.140103,0.113156,0.102145),dmax:DVec3::new(0.008061,0.003409,0.001442),input_floor:scan.input_floor};
        let roll=RollSettings{gamma:0.65,balance:Balance{warmth:-0.41,tint:-0.01},exposure:0.};
        let pipe=Pipeline::new(cal,roll,PhotoSettings::default())?;
        let area=Rect{x:192,y:96,width:8136,height:5424};scan.validate_area(area)?;
        println!("PHOTO {path}, fixed_WB={wb:?}, screenshot-rounded calibration/gamma balance, assumed photo WB0. Equal diagnostic crop={area:?}. 4399 is a second same-roll stress case, NOT a provided user grade.");
        let mut base=Vec::new();let mut ds=Vec::new();let mut anchor_indices=Vec::new();let mut floors=0;
        for y in 0..HEIGHT {for x in 0..WIDTH {
            let sx=area.x+(2*x+1)*area.width/(2*WIDTH);let sy=area.y+(2*y+1)*area.height/(2*HEIGHT);
            let s=scan.pixel(sx,sy)?;let p=pipe.apply(s)?;
            floors+=usize::from(p.floored.into_iter().any(|v|v));
            if (4700..4900).contains(&sx)&&(4200..4400).contains(&sy) {anchor_indices.push(base.len());}
            base.push(p.rgb);ds.push(density(s,cal));
        }}
        let anchor:Vec<_>=anchor_indices.iter().map(|&i|base[i]).collect();
        let goal=render_y(&renderer,&anchor,3.)?;
        println!("floored={floors}/{} single comparison patch=({} sampled points) current_EV3_mean_display_Y={goal:.9}",base.len(),anchor.len());
        for name in ["current","lift-linear","lift-radius","lift-radius-toe","lift-radius-black"] {
            let working=match candidate(name,&base,&ds,white,white_code,ceiling) {Ok(v)=>v,Err(e)=>{println!("CANDIDATE_REJECTED {name}: {e}");continue;}};
            let anchor:Vec<_>=anchor_indices.iter().map(|&i|working[i]).collect();
            let ev=if name=="current" {3.} else {exposure_match(&renderer,&anchor,goal)?};
            let gain=ev.exp2();
            let output=renderer.render(&working.iter().map(|v|*v*gain).collect::<Vec<_>>())?;
            let mut exact_black=0;let mut below_code=0;let mut max_encode=Vec::new();
            for p in &output {exact_black+=usize::from(p.max_element()==0.);let code=OutputTarget::DisplayP3.encode(p.max_element() as f64)?;below_code+=usize::from(code<1./255.);max_encode.push(code);}
            max_encode.sort_by(f64::total_cmp);
            println!("mode={name} matched_EV={ev:.6} patch_display_Y={:.9} exact_black={exact_black} max_encoded_below_1of255={below_code} max_encoded_p05_p50_p95=({:.6},{:.6},{:.6}) working_max={:.6}",render_y(&renderer,&anchor,ev)?,max_encode[max_encode.len()/20],max_encode[max_encode.len()/2],max_encode[max_encode.len()*19/20],working.iter().map(|v|v.abs().max_element()*gain).fold(0f64,f64::max));
            for (tag,offset) in [("minus1",-1.),("matched",0.),("plus1",1.)] {
                save(&format!("{DIR}/DSC{id:05}-{name}-{tag}.tif"),&working,ev+offset,OutputTarget::DisplayP3)?;
            }
            // Compare same original EV3 as well: matching is an explicit experimental adjustment, not hidden state.
            if name!="current" {save(&format!("{DIR}/DSC{id:05}-{name}-originalEV3.tif"),&working,3.,OutputTarget::DisplayP3)?;}
        }
    }
    Ok(())
}
