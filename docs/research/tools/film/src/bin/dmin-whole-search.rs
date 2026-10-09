use antitone::{raw::{RawFile,Rect},color::ap0_to_ap1,sampling,export::{self,ImageSize,Overwrite},rendering::OutputTarget};
use glam::DVec3;
use std::{error::Error,fs,path::Path};
type Result<T>=std::result::Result<T,Box<dyn Error>>;
const DIR:&str=".cache/probes/dmin-trace/whole-search2";
fn score(camera:DVec3)->f64 {(camera.x*camera.y*camera.z).cbrt()}
#[derive(Clone,Copy)]
struct Candidate {area:Rect,camera:DVec3,score:f64}
fn inside(r:Rect,outer:Rect)->bool {r.x>=outer.x&&r.y>=outer.y&&r.x+r.width<=outer.x+outer.width&&r.y+r.height<=outer.y+outer.height}
fn pixels(raw:&RawFile,r:Rect)->Result<Vec<DVec3>> {
    let image=raw.camera_image()?;let mut out=Vec::with_capacity((r.width*r.height) as usize);
    for y in r.y..r.y+r.height {for x in r.x..r.x+r.width {out.push(image.pixel(x,y)?);}}Ok(out)
}
fn main()->Result<()> {
    fs::create_dir_all(DIR)?;
    let mut raw=RawFile::open(Path::new("samples/DSC04393.ARW"))?;
    let wb=[2133.,1024.,1771.,1024.];let active=raw.metadata.image_area()?;let matrix=raw.metadata.camera_to_ap0()?;
    raw.develop_camera(wb)?;
    let picture=Rect{x:192,y:96,width:8136,height:5424};
    let width=active.width/32;let height=active.height/32;
    assert_eq!(width*32,active.width);assert_eq!(height*32,active.height);
    let image=raw.camera_image()?;
    let mut candidates=Vec::new();let mut hist=vec![vec![0_u64;65536];3];
    let mut maximum=DVec3::ZERO;let mut pixel_best=Candidate{area:Rect{x:0,y:0,width:1,height:1},camera:DVec3::ZERO,score:-1.};
    for ty in 0..height {for tx in 0..width {
        let r=Rect{x:active.x+tx*32,y:active.y+ty*32,width:32,height:32};let mut sum=DVec3::ZERO;
        for y in r.y..r.y+32 {for x in r.x..r.x+32 {
            let c=image.pixel(x,y)?;sum+=c;maximum=maximum.max(c);
            for channel in 0..3 {hist[channel][(c[channel]*65535.).round() as usize]+=1;}
            let rank=score(c);
            if rank>pixel_best.score {pixel_best=Candidate{area:Rect{x,y,width:1,height:1},camera:c,score:rank};}
        }}
        let camera=sum/1024.;candidates.push(Candidate{area:r,camera,score:score(camera)});
    }}
    let count=active.width as u64*active.height as u64;
    let mut percentiles=Vec::new();
    for p in [0.99,0.999,0.9999] {
        let rgb=DVec3::from_array(std::array::from_fn(|c|{
            let target=(count as f64*p).ceil() as u64;let mut sum=0;
            for(i,n) in hist[c].iter().enumerate(){sum+=n;if sum>=target{return i as f64/65535.;}}1.
        }));percentiles.push((p,rgb));
    }
    candidates.sort_by(|a,b|b.score.total_cmp(&a.score));
    println!("FULL ACTIVE IMAGE SEARCH {active:?} decoded_pixels={count} full_resolution_32x32_tiles={} wb={wb:?}; rank=geometric_mean(camera_RGB); NO edge reference, inversion, colour matrix or rendering used during selection",candidates.len());
    println!("CAMERA_CHANNEL_UPPER_TAIL (separate channel statistics, NOT a composite Dmin reference): percentiles={percentiles:?}; maximum={maximum:?}");
    println!("BRIGHTEST_SINGLE_PIXEL (NOT selected as Dmin) area={:?} score={} camera={:?} AP0={:?}",pixel_best.area,pixel_best.score,pixel_best.camera,matrix*pixel_best.camera);
    let mut refined=Vec::new();
    // Refine grid alignment locally without storing full-resolution integral images.
    let starts:Vec<_>=candidates.iter().filter(|c|inside(c.area,picture)).take(16).copied().collect();
    for initial in starts {for dy in [-16,-8,0,8,16] {for dx in [-16,-8,0,8,16] {
        let x=initial.area.x as i64+dx;let y=initial.area.y as i64+dy;
        if x<0||y<0 {continue;}
        let r=Rect{x:x as u32,y:y as u32,width:32,height:32};if !inside(r,picture){continue;}
        let camera=sampling::mean(&pixels(&raw,r)?)?;refined.push(Candidate{area:r,camera,score:score(camera)});
    }}}
    refined.sort_by(|a,b|b.score.total_cmp(&a.score));
    let mut csv=String::from("scope,rank,x,y,width,height,score,camera_r,camera_g,camera_b,ap0_r,ap0_g,ap0_b\n");
    for(scope,list) in [("active",&candidates),("picture-refined",&refined)] {
        for(i,c) in list.iter().take(24).enumerate() {
            let ap0=matrix*c.camera;
            csv.push_str(&format!("{scope},{},{},{},{},{},{},{},{},{},{},{},{}\n",i+1,c.area.x,c.area.y,c.area.width,c.area.height,c.score,c.camera.x,c.camera.y,c.camera.z,ap0.x,ap0.y,ap0.z));
        }
    }
    let mut selected=None;
    for c in &refined {
        let sensor=raw.region_sensor_diagnostics(c.area)?;
        println!("CANDIDATE region={:?} score={} camera={:?} AP0={:?} sensor={sensor:?}",c.area,c.score,c.camera,matrix*c.camera);
        if sensor.below_black.iter().chain(&sensor.at_white).any(|v|*v>0) {continue;}
        selected=Some(*c);break;
    }
    let selected=selected.ok_or("no usable whole-picture candidate")?;
    for n in [16,32,64,128] {
        let r=Rect{x:selected.area.x+16-n/2,y:selected.area.y+16-n/2,width:n,height:n};
        let camera=pixels(&raw,r)?;let mean=sampling::mean(&camera)?;
        let sd=(camera.iter().fold(DVec3::ZERO,|a,v|a+(*v-mean)*(*v-mean))/camera.len() as f64).sqrt();
        println!("SIZE_SENSITIVITY area={r:?} camera_mean={mean:?} CV={:?} AP0_mean={:?} joint_score={}",sd/mean,matrix*mean,score(mean));
    }
    for n in [64,128] {
        let best=candidates.iter().filter_map(|c| {
            let r=Rect{x:c.area.x,y:c.area.y,width:n,height:n};
            if !inside(r,picture){return None;}Some(r)
        }).take(64).map(|r| {
            let camera=sampling::mean(&pixels(&raw,r)?)?;
            Ok(Candidate{area:r,camera,score:score(camera)})
        }).collect::<Result<Vec<_>>>()?.into_iter().max_by(|a,b|a.score.total_cmp(&b.score)).ok_or("no larger candidate")?;
        println!("BRIGHT_LARGER_PATCH (evaluated at 64 strongest grid starts) size={n} area={:?} camera={:?} AP0={:?} score={}",best.area,best.camera,matrix*best.camera,best.score);
    }
    let base=matrix*selected.camera;
    println!("SELECTED WHOLE_PICTURE_ESTIMATE area={:?} RGB={base:?}; not proof of unexposed film/zero scene exposure",selected.area);
    fs::write(format!("{DIR}/whole-picture-candidates.csv"),csv)?;
    fs::write(format!("{DIR}/whole-picture-dmin.txt"),format!("{} {} {} {} {:.17} {:.17} {:.17}\n",selected.area.x,selected.area.y,selected.area.width,selected.area.height,base.x,base.y,base.z))?;
    let r=Rect{x:selected.area.x-192,y:selected.area.y-160,width:416,height:352};
    let source:Vec<_>=pixels(&raw,r)?.iter().map(|c|ap0_to_ap1()*matrix*(*c)*4.).collect();
    export::tiff(Path::new(&format!("{DIR}/whole-picture-candidate-source-plus2ev.tif")),ImageSize{width:r.width,height:r.height},OutputTarget::DisplayP3,Overwrite::Refuse,|x,y|Ok(source[(y*r.width+x) as usize]))?;
    Ok(())
}
