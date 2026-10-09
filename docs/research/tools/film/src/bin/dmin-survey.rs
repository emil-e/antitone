use antitone::{color::ap0_to_ap1, export::{self, ImageSize, Overwrite}, raw::{RawFile, Rect}, sampling};
use glam::DVec3;
use std::{error::Error, fs, path::Path};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const DIR:&str=".cache/probes/dmin-trace/run1";
fn main()->Result<()> {
    fs::create_dir_all(DIR)?;
    let wb=RawFile::open(Path::new("samples/DSC04378.ARW"))?.metadata.cam_mul;
    let mut raw=RawFile::open(Path::new("samples/DSC04393.ARW"))?;
    let matrix=raw.metadata.camera_to_ap0()?;
    let area=raw.metadata.image_area()?;
    println!("SOURCE DSC04393 fixed_WB={wb:?} as_shot_WB={:?} area={area:?} camera_to_AP0={matrix:?}",raw.metadata.cam_mul);
    raw.develop_camera(wb)?;
    let image=raw.camera_image()?;
    println!("DEVELOPED {}x{} warnings={} pre_mul={:?} maximum={} data_maximum={}",image.width,image.height,image.warnings,image.pre_mul,image.maximum,image.data_maximum);
    let patches=[
        ("full",area,ImageSize{width:1200,height:800}),
        ("top-centre",Rect{x:3600,y:12,width:1024,height:312},ImageSize{width:1024,height:312}),
        ("bottom-centre",Rect{x:3600,y:5460,width:1024,height:312},ImageSize{width:1024,height:312}),
        ("left",Rect{x:12,y:2300,width:400,height:800},ImageSize{width:400,height:800}),
        ("right",Rect{x:8250,y:2300,width:400,height:800},ImageSize{width:400,height:800}),
    ];
    for (name,r,size) in patches {
        let mut input=Vec::with_capacity((size.width*size.height) as usize);
        for y in 0..size.height {for x in 0..size.width {
            let sx=r.x+((2*x+1) as u64*r.width as u64/(2*size.width) as u64) as u32;
            let sy=r.y+((2*y+1) as u64*r.height as u64/(2*size.height) as u64) as u32;
            input.push(ap0_to_ap1()*matrix*raw.camera_image()?.pixel(sx,sy)?*4.);
        }}
        // +2 EV is solely for viewing the negative; all survey measurements below are unscaled.
        export::tiff(Path::new(&format!("{DIR}/source-{name}-inspection-plus2ev.tif")),size,antitone::rendering::OutputTarget::DisplayP3,Overwrite::Refuse,|x,y|Ok(input[(y*size.width+x) as usize]))?;
    }
    let mut candidates=Vec::new();
    for x in [256,1000,2000,3000,4000,5000,6000,7000,8000] {
        for y in [16,40,64,96,128,160,5510,5580,5644,5680,5710,5730] {
            candidates.push(Rect{x,y,width:128,height:16});
        }
    }
    for x in [16,36,64,128,8520,8560,8590,8610] {for y in [500,1500,2500,3500,4500] {
        candidates.push(Rect{x,y,width:16,height:64});
    }}
    let mut csv=String::from("x,y,width,height,camera_r,camera_g,camera_b,ap0_r,ap0_g,ap0_b,cv_r,cv_g,cv_b,below_black,at_white\n");
    for r in candidates {
        let diagnostics=raw.region_sensor_diagnostics(r)?;
        let mut pixels=Vec::with_capacity((r.width*r.height) as usize);
        for y in r.y..r.y+r.height {for x in r.x..r.x+r.width {pixels.push(raw.camera_image()?.pixel(x,y)?);}}
        let mean=sampling::mean(&pixels)?;
        let variance=pixels.iter().fold(DVec3::ZERO,|a,v|a+(*v-mean)*(*v-mean))/pixels.len() as f64;
        let cv=variance.sqrt()/mean;
        let ap0=matrix*mean;
        let black=diagnostics.below_black.iter().sum::<u64>();let white=diagnostics.at_white.iter().sum::<u64>();
        csv.push_str(&format!("{},{},{},{},{:.12},{:.12},{:.12},{:.12},{:.12},{:.12},{:.8},{:.8},{:.8},{black},{white}\n",r.x,r.y,r.width,r.height,mean.x,mean.y,mean.z,ap0.x,ap0.y,ap0.z,cv.x,cv.y,cv.z));
    }
    fs::write(format!("{DIR}/margin-survey.csv"),csv)?;
    Ok(())
}
