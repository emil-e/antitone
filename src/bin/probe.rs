use antitone::processing::{Calibration, Inversion, RollSettings};
use glam::DVec3;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let calibration = Calibration {
        dmin: DVec3::ONE,
        dmax: DVec3::splat(0.01),
        input_floor: 1e-6,
    };
    let roll = RollSettings::default();
    let inversion = Inversion::new(calibration, roll)?;
    println!(
        "Antitone processing model {} — synthetic numerical probe",
        roll.response.processing_model()
    );
    println!("The input floor below is a synthetic test value, not a validated RAW default.");
    for scan in [
        calibration.dmin,
        DVec3::splat(0.1),
        calibration.dmax,
        DVec3::splat(1.1),
        DVec3::ZERO,
    ] {
        let pixel = inversion.apply(scan)?;
        println!(
            "scan={scan:?} positive={:?} floored={:?}",
            pixel.rgb, pixel.floored
        );
    }
    Ok(())
}
