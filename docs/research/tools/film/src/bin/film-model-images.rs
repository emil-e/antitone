use antitone::{
    color::{Balance, ap0_to_ap1},
    export::{self, ImageSize, Overwrite},
    processing::{Calibration, Inversion, RollSettings},
    raw::{RawFile, Rect},
    rendering::{CpuRenderer, OutputTarget},
    sampling,
    scan::Scan,
};
use glam::{DMat2, DVec2, DVec3};
use std::{error::Error, fs, path::Path};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const WHOLE_DMIN: DVec3 = DVec3::new(0.16856635015466970, 0.13359162596075455, 0.11715123642832656);
const CLEAR_DMIN: DVec3 = DVec3::new(0.16551166738395376, 0.13368683444783128, 0.11901288348757422);
const WIDTH: u32 = 900;
const HEIGHT: u32 = 600;
const WHITE_PATCH: Rect = Rect { x: 3200, y: 3700, width: 64, height: 64 };
const MODELS: [Model; 3] = [
    Model { name: "current", beta: 1., shoulder: 0. },
    Model { name: "fitted-toe", beta: 2.5, shoulder: 0. },
    Model { name: "gentle-shoulder", beta: 2.5, shoulder: 0.02 },
];

#[derive(Clone, Copy, Debug)]
struct Model { name: &'static str, beta: f64, shoulder: f64 }
struct Curve { model: Model, cal: Calibration, gammas: DVec3, anchors: DVec3, legacy: Option<Inversion> }

fn log_ratio(a: f64, b: f64) -> f64 {
    let relative = (a - b) / b;
    if relative.abs() < 0.5 { relative.ln_1p() } else { a.ln() - b.ln() }
}
fn log_positive_expm1(a: f64) -> f64 {
    if a > 50. { a + (-(-a).exp()).ln_1p() } else { a.exp_m1().ln() }
}
impl Curve {
    fn new(model: Model, cal: Calibration, roll: RollSettings) -> Result<Self> {
        cal.validate()?;
        let gammas = roll.channel_gammas()?;
        if !(model.beta > 0. && model.beta.is_finite() && model.shoulder >= 0. && model.shoulder.is_finite()) {
            return Err("invalid experimental shape".into());
        }
        let mut anchors = DVec3::ZERO;
        for c in 0..3 {
            let floor_d = log_ratio(cal.dmin[c], cal.input_floor) / std::f64::consts::LN_10;
            if gammas[c] - model.shoulder * floor_d <= 0. {
                return Err("experimental shoulder pole falls within scan-floor domain".into());
            }
            let d = log_ratio(cal.dmin[c], cal.dmax[c]) / std::f64::consts::LN_10;
            anchors[c] = model.beta * std::f64::consts::LN_10 * d / (gammas[c] - model.shoulder * d);
        }
        let legacy = if model.beta == 1. && model.shoulder == 0. { Some(Inversion::new(cal, roll)?) } else { None };
        Ok(Self { model, cal, gammas, anchors, legacy })
    }
    fn apply(&self, scan: DVec3) -> Result<DVec3> {
        if let Some(legacy) = &self.legacy { return Ok(legacy.apply(scan)?.rgb); }
        if !scan.is_finite() { return Err("nonfinite scan".into()); }
        let mut rgb = DVec3::ZERO;
        for c in 0..3 {
            let s = scan[c].max(self.cal.input_floor);
            let d = log_ratio(self.cal.dmin[c], s) / std::f64::consts::LN_10;
            let denominator = self.gammas[c] - self.model.shoulder * d;
            if denominator <= 0. { return Err("pixel outside shoulder inverse domain".into()); }
            let a = self.model.beta * std::f64::consts::LN_10 * d / denominator;
            if a == 0. { rgb[c] = 0.; continue; }
            let log_n = if a > 0. { log_positive_expm1(a) } else { (-a.exp_m1()).ln() };
            // Signed-root extension is an explicit diagnostic policy, not film physics.
            // No below-base value is clipped or moved above zero in working data.
            rgb[c] = a.signum() * ((log_n - log_positive_expm1(self.anchors[c])) / self.model.beta).exp();
        }
        if !rgb.is_finite() { return Err("experimental inversion overflow".into()); }
        Ok(rgb)
    }
}
fn roll(balance: Balance) -> RollSettings { RollSettings { gamma: 0.65, balance, exposure: 0. } }
fn patch(scan: &mut Scan, area: Rect) -> Result<Vec<DVec3>> {
    scan.reference(area)?; // The app's full-resolution sensor validity check.
    let mut values = Vec::with_capacity((area.width * area.height) as usize);
    for y in area.y..area.y + area.height { for x in area.x..area.x + area.width {
        let value = scan.pixel(x, y)?;
        if !value.is_finite() || value.min_element() < scan.input_floor { return Err("unusable full-resolution patch".into()); }
        values.push(value);
    }}
    Ok(values)
}
fn fit_balance(cal: Calibration, model: Model, pixels: &[DVec3]) -> Result<Balance> {
    let residual = |coordinates: DVec2| -> Result<DVec2> {
        let curve = Curve::new(model, cal, roll(Balance { warmth: coordinates.x, tint: coordinates.y }))?;
        let positives = pixels.iter().map(|v| curve.apply(*v)).collect::<Result<Vec<_>>>()?;
        let mean = sampling::mean(&positives)?;
        if mean.min_element() <= 1e-8 { return Err("neutral fit too near base or nonpositive".into()); }
        Ok(DVec2::new(mean.x.ln() - mean.y.ln(), mean.z.ln() - mean.y.ln()))
    };
    let mut x = DVec2::new(-0.41, -0.01);
    for _ in 0..32 {
        let r = residual(x)?;
        let mut jacobian = DMat2::ZERO;
        for axis in 0..2 {
            let mut lo = x; let mut hi = x;
            lo[axis] = (x[axis] - 1e-5).max(-1.);
            hi[axis] = (x[axis] + 1e-5).min(1.);
            *jacobian.col_mut(axis) = (residual(hi)? - residual(lo)?) / (hi[axis] - lo[axis]);
        }
        if jacobian.determinant().abs() < 1e-9 || !jacobian.is_finite() { return Err("ill-conditioned experimental neutral fit".into()); }
        if r.length() < 1e-9 { return Ok(Balance { warmth: x.x, tint: x.y }); }
        let delta = jacobian.inverse() * r;
        let mut accepted = false;
        for attempt in 0..16 {
            let candidate = x - 0.5_f64.powi(attempt) * delta;
            if candidate.abs().max_element() > 1. || !candidate.is_finite() { continue; }
            if let Ok(next) = residual(candidate) && next.length() < r.length() {
                x = candidate; accepted = true; break;
            }
        }
        if !accepted { return Err("experimental neutral fit has no improving bounded step".into()); }
    }
    Err("experimental neutral fit did not converge".into())
}
fn transform(curve: &Curve, scans: &[DVec3]) -> Result<Vec<DVec3>> {
    let basis = ap0_to_ap1();
    scans.iter().map(|s| Ok(basis * curve.apply(*s)?)).collect()
}
fn render_y(renderer: &CpuRenderer, input: &[DVec3], ev: f64) -> Result<f64> {
    let scaled: Vec<_> = input.iter().map(|v| *v * ev.exp2()).collect();
    let values = renderer.render(&scaled)?;
    let xyz = OutputTarget::DisplayP3.rgb_to_xyz();
    Ok(values.iter().map(|v| (xyz * v.as_dvec3()).y).sum::<f64>() / values.len() as f64)
}
fn match_exposure(renderer: &CpuRenderer, input: &[DVec3], goal: f64) -> Result<f64> {
    let mut lo = -16.; let mut hi = 16.;
    if goal <= 1e-8 || goal < render_y(renderer, input, lo)? || goal > render_y(renderer, input, hi)? {
        return Err("brightness reference cannot be matched".into());
    }
    for _ in 0..24 { let mid = (lo + hi) * 0.5;
        if render_y(renderer, input, mid)? < goal { lo = mid; } else { hi = mid; }
    }
    Ok((lo + hi) * 0.5)
}
fn save(path: &str, pixels: &[DVec3], size: ImageSize, ev: f64) -> Result<()> {
    export::tiff(Path::new(path), size, OutputTarget::DisplayP3, Overwrite::Refuse,
        |x,y| Ok(pixels[(y * size.width + x) as usize] * ev.exp2()))?;
    Ok(())
}
fn describe(renderer: &CpuRenderer, input: &[DVec3], width: usize, ev: f64) -> Result<String> {
    let output = renderer.render(&input.iter().map(|v| *v * ev.exp2()).collect::<Vec<_>>())?;
    let matrix = OutputTarget::DisplayP3.rgb_to_xyz();
    let mut codes = Vec::with_capacity(output.len());
    let mut blacks = 0; let mut low = 0; let mut high = 0;
    for pixel in &output {
        let code = OutputTarget::DisplayP3.encode(pixel.max_element() as f64)?;
        blacks += usize::from(pixel.max_element() == 0.);
        low += usize::from(code < 1. / 255.);
        high += usize::from(code > 0.99);
        codes.push(OutputTarget::DisplayP3.encode((matrix * pixel.as_dvec3()).y)?);
    }
    let mean = codes.iter().sum::<f64>() / codes.len() as f64;
    let sigma = (codes.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / codes.len() as f64).sqrt();
    let mut neighbor = 0.; let mut pairs = 0;
    for i in 0..codes.len() {
        if i % width + 1 < width { neighbor += (codes[i + 1] - codes[i]).powi(2); pairs += 1; }
        if i + width < codes.len() { neighbor += (codes[i + width] - codes[i]).powi(2); pairs += 1; }
    }
    codes.sort_by(f64::total_cmp);
    Ok(format!("n={} black={} max_code_below_1of255={} max_code_above_99pct={} grey_code_Y_p05_p50_p95=({:.6},{:.6},{:.6}) mean={:.6} sigma={:.6} adjacent_RMS={:.6}",
        input.len(), blacks, low, high, codes[codes.len()/20], codes[codes.len()/2], codes[codes.len()*19/20], mean, sigma, (neighbor / pairs as f64).sqrt()))
}
fn compare_black(renderer: &CpuRenderer, current: &[DVec3], candidate: &[DVec3], ev: f64) -> Result<String> {
    let old = renderer.render(&current.iter().map(|v| *v * 8.).collect::<Vec<_>>())?;
    let new = renderer.render(&candidate.iter().map(|v| *v * ev.exp2()).collect::<Vec<_>>())?;
    assert_eq!(old.len(), new.len());
    let mut newly_black = 0; let mut regained = 0; let mut old_max = 0_f64;
    let mut positive_black = 0;
    for i in 0..old.len() {
        if old[i].max_element() > 0. && new[i].max_element() == 0. {
            newly_black += 1;
            old_max = old_max.max(OutputTarget::DisplayP3.encode(old[i].max_element() as f64)?);
        }
        regained += usize::from(old[i].max_element() == 0. && new[i].max_element() > 0.);
        positive_black += usize::from(candidate[i].max_element() > 0. && new[i].max_element() == 0.);
    }
    Ok(format!("newly_black={newly_black} regained={regained} largest_previous_max_channel_code_for_new_black={old_max:.9e} positive_working_exact_black={positive_black}"))
}
fn raw_region(scan: &Scan, area: Rect) -> Result<Vec<DVec3>> {
    scan.validate_area(area)?;
    let mut pixels = Vec::with_capacity((area.width * area.height) as usize);
    for y in area.y..area.y + area.height { for x in area.x..area.x + area.width { pixels.push(scan.pixel(x,y)?); }}
    Ok(pixels)
}
fn self_check(cal: Calibration) -> Result<()> {
    let legacy_balance = Balance { warmth: -0.41, tint: -0.01 };
    let old = Inversion::new(cal, roll(legacy_balance))?;
    let mut max_roundtrip = 0_f64;
    for model in MODELS {
        let curve = Curve::new(model, cal, roll(legacy_balance))?;
        assert_eq!(curve.apply(cal.dmin)?, DVec3::ZERO);
        assert!((curve.apply(cal.dmax)? - DVec3::ONE).abs().max_element() < 1e-12);
        let mut previous = DVec3::splat(-f64::INFINITY);
        for i in 0..1001 {
            let d = (cal.dmin / cal.dmax).ln() / std::f64::consts::LN_10 * (i as f64 / 1000.);
            let s = cal.dmin * (-d * std::f64::consts::LN_10).exp();
            let result = curve.apply(s)?;
            assert!(result.is_finite() && result.min_element() >= -1e-12);
            assert!((result - previous).min_element() >= 0.);
            previous = result;
            if model.name == "current" { assert_eq!(result, old.apply(s)?.rgb); }
            if i > 0 { for c in 0..3 {
                let z = log_positive_expm1(curve.anchors[c]) + model.beta * result[c].ln();
                let a = z.max(0.) + (-z.abs()).exp().ln_1p();
                let v = a / (model.beta * std::f64::consts::LN_10);
                let recovered = curve.gammas[c] * v / (1. + model.shoulder * v);
                max_roundtrip = max_roundtrip.max((recovered - d[c]).abs());
            }}
        }
        assert!(curve.apply(cal.dmin * 1.01)?.max_element() < 0.);
        println!("SELF_CHECK model={} Dmin0 Dmax1 monotonic signed_above_base preserved", model.name);
    }
    assert!(max_roundtrip < 1e-10);
    println!("SELF_CHECK maximum inverse/forward density error={max_roundtrip:.9e}; beta1 uses unchanged library inversion exactly");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn calibration() -> Calibration {
        Calibration { dmin: WHOLE_DMIN, dmax: DVec3::new(0.008061, 0.003409, 0.001442), input_floor: 2.1630933622265155e-5 }
    }
    #[test]
    fn anchors_and_full_floor_domain() -> Result<()> {
        for dmin in [WHOLE_DMIN, CLEAR_DMIN] {
        let cal = Calibration { dmin, ..calibration() };
        self_check(cal)?;
        for model in MODELS {
            let curve = Curve::new(model, cal, roll(Balance { warmth: -0.41, tint: -0.01 }))?;
            let mut previous = DVec3::splat(-f64::INFINITY);
            for i in 0..4097 {
                let d = (cal.dmin / cal.input_floor).ln() / std::f64::consts::LN_10 * (i as f64 / 4096.);
                let s = cal.dmin * (-d * std::f64::consts::LN_10).exp();
                let q = curve.apply(s)?;
                assert!(q.is_finite() && (q - previous).min_element() >= 0.);
                previous = q;
            }
            for epsilon in [1e-12, 1e-9, 1e-6] {
                assert!(curve.apply(cal.dmin * (1. - epsilon))?.min_element() > 0.);
                assert!(curve.apply(cal.dmin * (1. + epsilon))?.max_element() < 0.);
            }
        }
        }
        Ok(())
    }
    #[test]
    fn shoulder_pole_is_rejected_not_clamped() {
        let settings = RollSettings { gamma: 0.1, balance: Balance { warmth: -0.41, tint: -0.01 }, exposure: 0. };
        assert!(Curve::new(MODELS[2], calibration(), settings).is_err());
        assert!(Curve::new(MODELS[1], calibration(), settings).is_ok());
    }
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let clear = match args.as_slice() {
        [] => false,
        [arg] if arg == "--clear-film" => true,
        _ => return Err("expected no arguments or --clear-film".into()),
    };
    let dir = if clear { ".cache/probes/film-model-images/run3-clear-dmin" } else { ".cache/probes/film-model-images/run2-whole-dmin" };
    fs::create_dir_all(dir)?;
    let fixed_wb = RawFile::open(Path::new("samples/DSC04378.ARW"))?.metadata.cam_mul;
    let mut ring = Scan::open(Path::new("samples/DSC04399.ARW"), Some(fixed_wb))?;
    let cal = Calibration { dmin: if clear { CLEAR_DMIN } else { WHOLE_DMIN }, dmax: DVec3::new(0.008061,0.003409,0.001442), input_floor: ring.input_floor };
    self_check(cal)?;
    let neutral = patch(&mut ring, WHITE_PATCH)?;
    println!("OFFLINE DIAGNOSTIC ONLY: corrected dog Dmin ({}), rounded screenshot Dmax, gamma .65, Photo WB0; not exact user grade. All models re-balanced through actual model. ACES unchanged; no black offset/flat-field correction. fixed WB={fixed_wb:?}; calibration={cal:?}; output={dir}", if clear { "measured clear film" } else { "whole-picture estimate" });
    println!("OTHER-FRAME LIMIT: dog-derived Dmin is a shared-roll diagnostic, not verified per-frame base/flat-field calibration; same-coordinate clear-film context is exported for inspection.");
    println!("COMMON ROLL NEUTRAL: DSC04399 visually white ring patch {WHITE_PATCH:?}; full-resolution pixels={}, not a laboratory target", neutral.len());
    let mut balances = Vec::new();
    for model in MODELS {
        let balance = fit_balance(cal, model, &neutral)?;
        let curve = Curve::new(model, cal, roll(balance))?;
        let mean = sampling::mean(&neutral.iter().map(|p| curve.apply(*p)).collect::<Result<Vec<_>>>()?)?;
        println!("FIT {model:?} balance={balance:?} actual_gammas={:?} full_patch_AP0_mean={mean:?}", curve.gammas);
        balances.push(balance);
    }
    drop(ring);
    let ids: &[u32] = if clear { &[4393] } else { &[4393,4399,4415] };
    for &id in ids {
        let path = format!("samples/DSC{id:05}.ARW");
        let mut scan = Scan::open(Path::new(&path), Some(fixed_wb))?;
        if (scan.input_floor - cal.input_floor).abs() > 1e-12 { return Err("scan development floor differs".into()); }
        let area = Rect { x: 192, y: 96, width: 8136, height: 5424 };
        scan.validate_area(area)?;
        let base_area = Rect { x: 5000, y: 5660, width: 128, height: 32 };
        println!("BASE_PROBE photo={id} same_coordinate_area={base_area:?} measured_AP0={:?}; context must be inspected before calling other frames clear film",scan.reference(base_area)?);
        let context_area = Rect { x: 4840, y: 5540, width: 512, height: 216 };
        let context: Vec<_> = raw_region(&scan,context_area)?.iter().map(|v| ap0_to_ap1() * *v).collect();
        save(&format!("{dir}/DSC{id:05}-base-context-source-plus2EV.tif"),&context,ImageSize{width:512,height:216},2.)?;
        let anchor_area = if id == 4415 { Rect { x: 4000, y: 3000, width: 128, height: 128 } }
            else { Rect { x: 4700, y: 4200, width: 128, height: 128 } };
        let anchor = patch(&mut scan, anchor_area)?;
        let mut source = Vec::with_capacity((WIDTH * HEIGHT) as usize);
        let mut floors = 0; let mut above = [0_usize;3];
        for y in 0..HEIGHT { for x in 0..WIDTH {
            let s = scan.pixel(area.x + (2*x+1)*area.width/(2*WIDTH), area.y + (2*y+1)*area.height/(2*HEIGHT))?;
            floors += usize::from(s.min_element() < cal.input_floor);
            for c in 0..3 { above[c] += usize::from(s[c] > cal.dmin[c]); }
            source.push(s);
        }}
        let regions = if id == 4393 { vec![
            ("forehead",Rect { x:5750,y:2400,width:512,height:384 }),
            ("muzzle",Rect { x:5100,y:3350,width:512,height:384 }),
            ("chest",Rect { x:6650,y:4200,width:512,height:384 }),
            ("sky",Rect { x:1800,y:400,width:512,height:384 }),
        ] } else if id == 4399 { vec![
            ("red-ring",Rect { x:4100,y:1800,width:512,height:384 }),
            ("white-ring",Rect { x:WHITE_PATCH.x,y:WHITE_PATCH.y,width:256,height:192 }),
            ("foliage-shadow",Rect { x:1800,y:4300,width:512,height:384 }),
        ] } else { vec![
            ("sunlit-canopy",Rect { x:3500,y:1000,width:512,height:384 }),
            ("sky-bokeh",Rect { x:6000,y:1500,width:512,height:384 }),
            ("leaf-shadow",Rect { x:4700,y:3500,width:512,height:384 }),
        ] };
        let rois = regions.iter().map(|(_,r)| raw_region(&scan,*r)).collect::<Result<Vec<_>>>()?;
        let renderer = CpuRenderer::new(OutputTarget::DisplayP3)?;
        let base_curve = Curve::new(MODELS[0],cal,roll(balances[0]))?;
        let goal = render_y(&renderer, &transform(&base_curve,&anchor)?, 3.)?;
        let current_frame = transform(&base_curve,&source)?;
        let current_rois = rois.iter().map(|r| transform(&base_curve,r)).collect::<Result<Vec<_>>>()?;
        println!("PHOTO {path} crop={area:?}; output={WIDTH}x{HEIGHT} nearest centres; floored={floors}; scan_above_Dmin={above:?}; brightness patch={anchor_area:?} full-resolution points={} reference_current_EV3_Y={goal:.9}",anchor.len());
        for (i,model) in MODELS.iter().enumerate() {
            let curve = Curve::new(*model,cal,roll(balances[i]))?;
            let input = transform(&curve,&source)?;
            let anchor_input = transform(&curve,&anchor)?;
            let ev = if i==0 {3.} else {match_exposure(&renderer,&anchor_input,goal)?};
            let matched_y = render_y(&renderer,&anchor_input,ev)?;
            assert!((matched_y-goal).abs() < 1e-8 + goal*5e-6);
            println!("GRADE photo={id} model={} balance={:?} matched_EV={ev:.9} brightness_Y={matched_y:.9} brightness_relative_error={:.9e}",model.name,balances[i],(matched_y-goal)/goal);
            println!("BLACK_CHANGE photo={id} model={} area=frame {}",model.name,compare_black(&renderer,&current_frame,&input,ev)?);
            let mut last_mean = -f64::INFINITY;
            for offset in [-2.,-1.,0.,1.,2.] {
                let mean = render_y(&renderer,&input,ev+offset)?;
                assert!(mean >= last_mean); last_mean = mean;
                println!("FRAME photo={id} model={} offset={offset:+} {} mean_display_Y={mean:.9}",model.name,describe(&renderer,&input,WIDTH as usize,ev+offset)?);
                let tag=if offset==0. {"matched".to_owned()} else {format!("ev{offset:+}")};
                save(&format!("{dir}/DSC{id:05}-{}-{tag}.tif",model.name),&input,ImageSize{width:WIDTH,height:HEIGHT},ev+offset)?;
            }
            for (r,((name,rect),scans)) in regions.iter().zip(&rois).enumerate() {
                let pixels = transform(&curve,scans)?;
                println!("ROI photo={id} model={} area={name} {rect:?} {}",model.name,describe(&renderer,&pixels,rect.width as usize,ev)?);
                println!("BLACK_CHANGE photo={id} model={} area={name} {}",model.name,compare_black(&renderer,&current_rois[r],&pixels,ev)?);
                save(&format!("{dir}/DSC{id:05}-{}-{name}-100pct.tif",model.name),&pixels,ImageSize{width:rect.width,height:rect.height},ev)?;
            }
        }
    }
    Ok(())
}
