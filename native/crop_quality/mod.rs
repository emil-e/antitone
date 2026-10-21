//! Read-only crop-quality diagnostics. Marks express independent observations, not accepted edits.
use crate::{
    auto_crop,
    coverage::Coverage,
    crop::Crop,
    geometry::SourceRegion,
    project::{Project, SourceId},
    scan_regions::Quad,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[cfg(all(feature = "editor", target_os = "macos"))]
mod inspection;
mod tests;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    cases: Vec<Case>,
    margins_percent: Vec<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    project: PathBuf,
    source: SourceId,
    observation: Observation,
    #[serde(default)]
    marks: Marks,
    #[serde(default)]
    windows: Vec<[u32; 4]>,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Observation {
    VisibleFrame,
    Censored,
    Ambiguous,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marks {
    #[serde(default)]
    wanted: Vec<Quad>,
    #[serde(default)]
    unwanted: Vec<Quad>,
    #[serde(default)]
    dense_wanted: Vec<Quad>,
}

struct CaseInput {
    spec: Case,
    project_path: PathBuf,
    project: Project,
}

#[derive(Serialize)]
struct Revision {
    locator: PathBuf,
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
}

#[derive(Default)]
struct Inputs(Vec<Revision>);

impl Inputs {
    fn remember(&mut self, locator: &Path) -> Result<PathBuf> {
        let path = locator.canonicalize()?;
        let metadata = fs::metadata(&path)?;
        self.0.push(Revision {
            locator: locator.to_owned(),
            path: path.clone(),
            bytes: metadata.len(),
            modified: metadata.modified()?,
        });
        Ok(path)
    }

    fn verify(&self) -> Result<()> {
        for prior in &self.0 {
            let current = fs::metadata(&prior.path)?;
            if prior.locator.canonicalize()? != prior.path
                || current.len() != prior.bytes
                || current.modified()? != prior.modified
            {
                return Err(format!("diagnostic input changed: {}", prior.path.display()).into());
            }
        }
        Ok(())
    }
}

fn load(path: &Path, inputs: &mut Inputs) -> Result<(Vec<CaseInput>, Vec<f64>)> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let target = inputs.remember(&path)?;
    let manifest: Manifest = serde_json::from_slice(&fs::read(target)?)?;
    if !(1..=3).contains(&manifest.cases.len())
        || !(1..=8).contains(&manifest.margins_percent.len())
    {
        return Err("diagnostic requires 1–3 cases and 1–8 margins".into());
    }
    let mut margins = HashSet::new();
    for &margin in &manifest.margins_percent {
        if !margin.is_finite()
            || !(0.0..=auto_crop::MAX_MARGIN_PERCENT).contains(&margin)
            || !margins.insert(if margin == 0. { 0 } else { margin.to_bits() })
        {
            return Err("invalid or duplicate crop margin".into());
        }
    }
    let mut identities = HashSet::new();
    let mut cases = vec![];
    for spec in manifest.cases {
        // Canonical targets identify inputs, but relative sources use the normal project locator.
        let project_path = path.parent().unwrap().join(&spec.project);
        let target = inputs.remember(&project_path)?;
        if !identities.insert((target.clone(), spec.source)) {
            return Err("duplicate project/source case".into());
        }
        let project = Project::load(&target)?;
        let active = project.source(spec.source)?.development.active_area.into();
        for marks in [
            &spec.marks.wanted,
            &spec.marks.unwanted,
            &spec.marks.dense_wanted,
        ] {
            if marks.len() > 128 {
                return Err("at most 128 quadrilaterals per mark category".into());
            }
            for &quad in marks {
                crate::scan_regions::Areas::coverage(quad)?.bounds(active)?;
            }
        }
        if spec.windows.len() > 12 {
            return Err("at most 12 original inspection windows per case".into());
        }
        for &window in &spec.windows {
            let window: SourceRegion = window.into();
            window.validate()?;
            if window.width > 256 || window.height > 256 || !active.contains_region(window) {
                return Err(
                    "inspection window must fit the active source and be at most 256x256".into(),
                );
            }
        }
        cases.push(CaseInput {
            spec,
            project_path,
            project,
        });
    }
    inputs.verify()?;
    Ok((cases, manifest.margins_percent))
}

fn output() -> Result<PathBuf> {
    let path =
        PathBuf::from(std::env::var_os("ANTITONE_CROP_OUTPUT").ok_or("ANTITONE_CROP_OUTPUT")?);
    let parent = path
        .parent()
        .ok_or("output parent required")?
        .canonicalize()?;
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".cache")
        .canonicalize()?;
    if !parent.starts_with(cache) {
        return Err("output must be under this checkout's .cache".into());
    }
    Ok(parent.join(path.file_name().ok_or("output name required")?))
}

fn proposal(case: &CaseInput) -> Result<Crop> {
    let id = case.spec.source;
    let source = case.project.source(id)?;
    if source.reference_only || !case.project.order.contains(&id) {
        return Err("source is not an ordinary filmstrip photo".into());
    }
    let areas = case
        .project
        .analysis_regions
        .iter()
        .find(|entry| entry.source == id)
        .ok_or("retained analysis areas unavailable")?;
    Ok(auto_crop::propose(
        source.development.active_area.into(),
        &areas.areas,
    )?)
}

/// Clip the mark in crop-local axes; a source bounding box would count unseen corners.
fn overlap(crop: Crop, mark: Coverage) -> f64 {
    let mut polygon: Vec<_> = mark.corners().map(|p| crop.local_point(p)).into();
    for (axis, limit, greater) in [
        (0, 0., true),
        (0, f64::from(crop.size[0]), false),
        (1, 0., true),
        (1, f64::from(crop.size[1]), false),
    ] {
        if polygon.is_empty() {
            return 0.;
        }
        let inside = |p: [f64; 2]| {
            if greater {
                p[axis] >= limit
            } else {
                p[axis] <= limit
            }
        };
        let mut next = vec![];
        let mut previous = *polygon.last().unwrap();
        for &point in &polygon {
            if inside(previous) != inside(point) {
                let t = (limit - previous[axis]) / (point[axis] - previous[axis]);
                next.push(std::array::from_fn(|i| {
                    previous[i] + t * (point[i] - previous[i])
                }));
            }
            if inside(point) {
                next.push(point);
            }
            previous = point;
        }
        polygon = next;
    }
    if polygon.len() < 3 {
        return 0.;
    }
    let origin = polygon[0];
    polygon[1..]
        .windows(2)
        .map(|pair| {
            (pair[0][0] - origin[0]) * (pair[1][1] - origin[1])
                - (pair[0][1] - origin[1]) * (pair[1][0] - origin[0])
        })
        .sum::<f64>()
        .abs()
        / 2.
}

#[derive(Serialize)]
pub(crate) struct Score {
    marked_source_pixel_area: f64,
    error_source_pixel_area: Option<f64>,
}

pub(crate) fn score(crop: Crop, marks: &[Quad], wanted: bool) -> Result<Score> {
    let mut total = 0.;
    let mut included = 0.;
    for &quad in marks {
        let region = crate::scan_regions::Areas::coverage(quad)?;
        total += region.area();
        included += overlap(crop, region);
    }
    Ok(Score {
        marked_source_pixel_area: total,
        error_source_pixel_area: (total > 0.).then_some(if wanted {
            (total - included).max(0.)
        } else {
            included
        }),
    })
}

fn report(cases: &[CaseInput], margins: &[f64]) -> Result<Value> {
    let mut results = vec![];
    for case in cases {
        let source = case.project.source(case.spec.source)?;
        let active = source.development.active_area.into();
        let base = proposal(case).map_err(|error| error.to_string());
        let mut proposals = vec![];
        for &margin in margins {
            let crop = base.clone().and_then(|crop| {
                auto_crop::inward_margin(crop, active, margin).map_err(|e| e.to_string())
            });
            proposals.push(match crop {
                Ok(crop) => json!({
                    "margin_percent": margin,
                    "status": "complete_proposal",
                    "crop": crop,
                    "proposal_on_uncertain_source": case.spec.observation!=Observation::VisibleFrame,
                    "wanted_lost": score(crop,&case.spec.marks.wanted,true)?,
                    "unwanted_admitted": score(crop,&case.spec.marks.unwanted,false)?,
                    "dense_wanted_lost": score(crop,&case.spec.marks.dense_wanted,true)?,
                }),
                Err(reason) => json!({"margin_percent":margin,"status":"abstain","reason":reason}),
            });
        }
        results.push(json!({
            "project": case.project_path,
            "source": case.spec.source,
            "observation": case.spec.observation,
            "proposals": proposals,
        }));
    }
    Ok(json!({
        "limits": concat!(
            "Retained project evidence and normal crop/margin policy; no detector, RAW freshness check or state promotion. ",
            "Errors sum supplied marked polygon areas, not whole-picture accuracy; overlapping marks count repeatedly. ",
            "Missing marks have null error; abstention has no crop-error score. Observation labels are not inferred truth."
        ),
        "cases": results,
    }))
}

#[test]
#[ignore = "Explicit private project/mark manifest and fresh report directory; no RAW development or accepted-state changes"]
fn report_marked_crop_quality() -> Result<()> {
    let path =
        PathBuf::from(std::env::var_os("ANTITONE_CROP_MANIFEST").ok_or("ANTITONE_CROP_MANIFEST")?);
    let mut inputs = Inputs::default();
    let (cases, margins) = load(&path, &mut inputs)?;
    let mut report = report(&cases, &margins)?;
    inputs.verify()?;
    report["inputs"] = serde_json::to_value(&inputs.0)?;
    let out = output()?;
    fs::create_dir(&out)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}
