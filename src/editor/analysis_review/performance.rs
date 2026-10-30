//! Bounded, explicit private-roll observation of production area preparation, not UI latency.
use super::*;
use crate::{cache_observation, region_timing};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};

type Result<T> = std::result::Result<T, String>;
const LIMIT: Duration = Duration::from_secs(240);
const WARM_BUDGET: u64 = 600 << 20;

struct NativeEvent {
    file: String,
    phase: &'static str,
    at: Instant,
}

fn native_report(events: &[NativeEvent], origin: Instant) -> Value {
    let mut counts = BTreeMap::<&str, usize>::new();
    let mut seconds = BTreeMap::<&str, f64>::new();
    for event in events {
        if event.phase == "develop_begin" {
            *counts.entry(&event.file).or_default() += 1;
        }
    }

    for pair in events.windows(2) {
        let name = match (pair[0].phase, pair[1].phase) {
            ("open", "unpack_begin") => "open_validate",
            ("unpack_begin", "unpack_end") => "unpack",
            ("develop_begin", "develop_end") => "develop_validate",
            _ => continue,
        };
        assert_eq!(pair[0].file, pair[1].file);
        *seconds.entry(name).or_default() += pair[1].at.duration_since(pair[0].at).as_secs_f64();
    }
    json!({
        "developments": counts.values().sum::<usize>(),
        "developments_by_source": counts,
        "seconds": seconds,
        "events": events.iter().map(|event| json!({
            "source": event.file,
            "phase": event.phase,
            "start_seconds": event.at.duration_since(origin).as_secs_f64(),
        })).collect::<Vec<_>>()
    })
}

fn stage_report(timings: &region_timing::Timings, wall: Duration) -> Value {
    let mut stages = BTreeMap::<&str, Vec<&region_timing::Sample>>::new();
    for sample in &timings.samples {
        stages.entry(sample.stage).or_default().push(sample);
    }

    let rows: Vec<_> = stages
        .into_iter()
        .map(|(stage, samples)| {
            let mut values: Vec<_> = samples.iter().map(|s| s.seconds).collect();
            values.sort_by(f64::total_cmp);
            let percentile =
                |q: f64| values[((values.len() as f64 * q).ceil() as usize).saturating_sub(1)];
            let inclusive = values.iter().sum::<f64>();
            let exclusive = samples.iter().map(|s| s.exclusive_seconds).sum::<f64>();

            println!(
                "region_stage phase={stage} calls={} inclusive_seconds={inclusive:.6} exclusive_seconds={exclusive:.6} p50_seconds={:.6} p95_seconds={:.6}",
                values.len(),
                percentile(0.5),
                percentile(0.95),
            );
            json!({
                "stage": stage,
                "calls": values.len(),
                "inclusive_seconds": inclusive,
                "exclusive_seconds": exclusive,
                "p50_seconds": percentile(0.5),
                "p95_seconds": percentile(0.95),
                "max_seconds": values.last(),
            })
        })
        .collect();
    let accounted = timings
        .samples
        .iter()
        .map(|s| s.exclusive_seconds)
        .sum::<f64>();
    json!({
        "wall_seconds": wall.as_secs_f64(),
        "accounted_seconds": accounted,
        "residual_seconds": wall.as_secs_f64() - accounted,
        "stages": rows,
        "samples": timings.samples,
    })
}

#[test]
fn stage_summary_uses_exclusive_accounting_and_nearest_rank_source_percentiles() {
    let (_, mut timings) = region_timing::record(|| ());
    timings.samples = vec![
        region_timing::Sample {
            stage: "inner",
            source: None,
            start_seconds: 0.,
            seconds: 1.,
            exclusive_seconds: 1.,
        },
        region_timing::Sample {
            stage: "inner",
            source: None,
            start_seconds: 2.,
            seconds: 3.,
            exclusive_seconds: 3.,
        },
        region_timing::Sample {
            stage: "outer",
            source: None,
            start_seconds: 0.,
            seconds: 10.,
            exclusive_seconds: 6.,
        },
    ];
    let report = stage_report(&timings, Duration::from_secs(12));
    assert_eq!(report["accounted_seconds"], 10.);
    assert_eq!(report["residual_seconds"], 2.);
    assert_eq!(report["stages"][0]["calls"], 2);
    assert_eq!(report["stages"][0]["p50_seconds"], 1.);
    assert_eq!(report["stages"][0]["p95_seconds"], 3.);
    assert_eq!(
        stage_report(&region_timing::record(|| ()).1, Duration::from_secs(1))["residual_seconds"],
        1.
    );
}

#[test]
fn native_summary_counts_actual_developments_and_keeps_nested_phases_separate() {
    let origin = Instant::now();
    let events: Vec<_> = [
        ("open", 0),
        ("unpack_begin", 1),
        ("unpack_end", 3),
        ("develop_begin", 4),
        ("develop_end", 7),
        ("open", 8),
        ("unpack_begin", 9),
        ("unpack_end", 10),
        ("develop_begin", 11),
        ("develop_end", 13),
    ]
    .into_iter()
    .map(|(phase, seconds)| NativeEvent {
        file: "authored.ARW".into(),
        phase,
        at: origin + Duration::from_secs(seconds),
    })
    .collect();
    let report = native_report(&events, origin);
    assert_eq!(report["developments"], 2);
    assert_eq!(report["developments_by_source"]["authored.ARW"], 2);
    assert_eq!(report["seconds"]["open_validate"], 2.);
    assert_eq!(report["seconds"]["unpack"], 3.);
    assert_eq!(report["seconds"]["develop_validate"], 5.);
}

fn within_limit(start: Instant, monitor: &cache_observation::Monitor) -> Result<()> {
    monitor.check()?;
    if start.elapsed() >= LIMIT {
        return Err("region preparation diagnostic duration limit".into());
    }
    Ok(())
}

fn write_new(path: &Path, report: &Value) -> Result<()> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(report).unwrap())
        .map_err(|e| e.to_string())
}

#[test]
#[ignore = "Explicit one-roll native preparation timing; private read-only RAWs, resource stops, no window"]
fn profile_complete_roll_area_preparation() {
    let evidence = PathBuf::from(
        std::env::var_os("ANTITONE_REGION_EVIDENCE").expect("ANTITONE_REGION_EVIDENCE"),
    )
    .canonicalize()
    .unwrap();
    let raw_root = PathBuf::from(std::env::var_os("ANTITONE_RAW_ROOT").expect("ANTITONE_RAW_ROOT"))
        .canonicalize()
        .unwrap();
    let output = PathBuf::from(
        std::env::var_os("ANTITONE_REGION_OUTPUT").expect("fresh ANTITONE_REGION_OUTPUT"),
    );
    let name =
        std::env::var("ANTITONE_REGION_ROLL").expect("explicit complete ANTITONE_REGION_ROLL");
    let arm =
        std::env::var("ANTITONE_REGION_PERF_ARM").expect("ANTITONE_REGION_PERF_ARM=cold|warm");
    assert!(matches!(arm.as_str(), "cold" | "warm"));
    let parent = output.parent().unwrap().canonicalize().unwrap();
    let cache_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".cache")
        .canonicalize()
        .unwrap();
    assert!(
        parent.starts_with(&cache_root),
        "reports must stay in the task worktree .cache"
    );
    assert!(!parent.starts_with(&evidence) && !parent.starts_with(&raw_root));
    fs::create_dir(&output).expect("choose a fresh report directory");
    let manifest: Value =
        serde_json::from_slice(&fs::read(evidence.join("roll-input/rolls.json")).unwrap()).unwrap();
    let roll = manifest["rolls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .expect("complete retained roll");
    let paths: Vec<_> = roll["photos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let relative = Path::new(p.as_str().unwrap());
            assert!(
                relative
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)))
            );
            let path = raw_root.join(relative).canonicalize().unwrap();
            assert!(path.starts_with(&raw_root));
            path
        })
        .collect();
    assert!(
        (2..=40).contains(&paths.len()),
        "bounded complete roll only; never truncate a roll"
    );
    let revisions: Vec<_> = paths.iter().map(|p| thumbnails::FileKey::new(p)).collect();
    assert!(revisions.iter().all(|r| r.revision.is_some()));
    let comparison = std::env::var_os("ANTITONE_REGION_PERF_COMPARE")
        .map(|p| serde_json::from_slice::<Value>(&fs::read(p).unwrap()).unwrap());
    assert!(
        arm != "warm" || comparison.is_some(),
        "warm arm requires ANTITONE_REGION_PERF_COMPARE from the cold run"
    );
    let result = cache_observation::with_monitor(|monitor, device, _| {
        let start = Instant::now();
        let mut document = Document::default();
        let cache = Arc::new(crate::source::SourceCache::new(0));
        monitor.phase("region_headers", || {
            for path in &paths {
                within_limit(start, monitor)?;
                let (source, raw) = document::open_source(path, None).map_err(|e| e.to_string())?;
                cache_observation::source_bound(&raw.metadata)?;
                let wb = raw.metadata.as_shot_wb();
                document
                    .register_with_development(source, raw.metadata.development(wb))
                    .map_err(|e| e.to_string())?;
            }
            document.validate().map_err(|e| e.to_string())
        })?;
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = events.clone();
        let guard = monitor.clone();
        let scheduler = source_loading::SourceLoader::new_observed(move |path, phase, metadata| {
            within_limit(start, &guard)?;
            if let Some(m) = metadata {
                cache_observation::source_bound(m)?;
            }
            let phase = match phase {
                source_loading::NativePhase::Open => "open",
                source_loading::NativePhase::UnpackBegin => "unpack_begin",
                source_loading::NativePhase::UnpackEnd => "unpack_end",
                source_loading::NativePhase::DevelopBegin => "develop_begin",
                source_loading::NativePhase::DevelopEnd => "develop_end",
            };
            observed.lock().unwrap().push(NativeEvent {
                file: path.file_name().unwrap().to_string_lossy().into_owned(),
                phase,
                at: Instant::now(),
            });
            Ok(())
        });
        let work = (|| {
            let mut seed_report = Value::Null;
            if arm == "warm" {
                cache.set_budget(WARM_BUDGET);
                let id = *document.project.order.last().unwrap();
                let path = document.source_path(id).map_err(|e| e.to_string())?;
                let expected = document.project.source(id).unwrap();
                let wb = expected.development.decode.fixed_wb;
                let file = document.file_key(&path);
                let source = monitor.phase("region_seed_one_source", || {
                    source::Request {
                        input: document.project.input,
                        reference: Arc::default(),
                        path,
                        wb: Some(wb),
                        expected: Some(expected.clone()),
                        sources: Some(scheduler.provider(Some(cache.clone()))),
                        file,
                        correction: None,
                    }
                    .load(&scheduler, &|| start.elapsed() >= LIMIT)
                    .map(|developed| developed.context.source)
                })?;
                let bytes = source
                    .retained_bytes()
                    .ok_or("source accounting unavailable")?;
                assert!(bytes <= WARM_BUDGET);
                drop(source);
                assert_eq!(cache.charges().retained, bytes);
                seed_report = json!({"source":id,"bytes":bytes,"native":native_report(&events.lock().unwrap(),start)});
                events.lock().unwrap().clear();
            }
            let before = document.project.clone();
            let request = analysis::Request {
                revision: 1,
                selected: document.project.order[0],
                document,
                film: None,
                local_support: false,
                picture: None,
                areas: None,
            };
            let generation = AtomicU64::new(1);
            let stop = Mutex::new(None);
            let prepare_start = Instant::now();
            let (prepared, timings) = region_timing::record(|| {
                preparation::prepare_with_measurements(
                    &request,
                    &generation,
                    &scheduler,
                    device,
                    |_, _| {
                        if let Err(error) = within_limit(start, monitor) {
                            *stop.lock().unwrap() = Some(error);
                            generation.store(2, Ordering::Relaxed);
                        }
                    },
                    Default::default(),
                    &cache,
                )
            });
            let wall = prepare_start.elapsed();
            let native = native_report(&events.lock().unwrap(), prepare_start);
            let calls = |stage: &str| timings.samples.iter().filter(|s| s.stage == stage).count();
            let mut expected_by_source = BTreeMap::<String, u64>::new();
            for sample in timings
                .samples
                .iter()
                .filter(|s| matches!(s.stage, "coarse_acquire" | "fine_acquire"))
            {
                let id = SourceId(sample.source.expect("acquisition source context"));
                let path = request
                    .document
                    .source_path(id)
                    .map_err(|e| e.to_string())?;
                let expected = expected_by_source
                    .entry(path.file_name().unwrap().to_string_lossy().into_owned())
                    .or_default();
                if arm != "warm" || Some(&id) != request.document.project.order.last() {
                    *expected += 1;
                }
            }
            let expected_developments = expected_by_source.values().sum::<u64>();
            let entries = prepared
                .as_ref()
                .ok()
                .and_then(|v| v.as_ref())
                .map(|a| &a.entries);
            let report = json!({
                "roll":name,"arm":arm,"sources":paths.iter().map(|p|p.file_name().unwrap().to_string_lossy()).collect::<Vec<_>>(),
                "source_filesystem_cache":"uncontrolled; cold means empty application source store only",
                "limits":"Production provider only, no import/decode seed in preparation wall time, no UI/presentation or photographic acceptance. Native times are nested inside acquisition. Exclusive stage times must be used for accounting.",
                "files":revisions,
                "preparation":stage_report(&timings,wall),"native":native,"seed":seed_report,
                "expected_developments":expected_developments,"expected_by_source":expected_by_source,
                "optional_budget_bytes":if arm=="warm" {WARM_BUDGET} else {0},
                "retained_bytes_after":cache.charges().retained,
                "entries":entries,"error":prepared.as_ref().err().map(ToString::to_string),
            });
            write_new(&output.join("report.json"), &report)?;
            println!(
                "region_summary arm={arm} photos={} seconds={:.6} native_developments={} expected_native_developments={expected_developments:?} report={}",
                paths.len(),
                wall.as_secs_f64(),
                report["native"]["developments"],
                output.join("report.json").display()
            );
            if let Some(error) = stop.into_inner().unwrap() {
                return Err(error);
            }
            let approved = prepared
                .map_err(|e| e.to_string())?
                .ok_or("unexpected preparation cancellation")?;
            verify_file(&request.document, &approved).map_err(|e| e.to_string())?;
            assert_eq!(
                request.document.project, before,
                "preparation changed accepted state"
            );
            assert_eq!(approved.files, revisions);
            let developed = report["native"]["developments"].as_u64().unwrap() as usize;
            assert_eq!(developed as u64, expected_developments);
            assert_eq!(calls("coarse_acquire"), paths.len());
            for (index, path) in paths.iter().enumerate() {
                let file = path.file_name().unwrap().to_str().unwrap();
                let count = report["native"]["developments_by_source"][file]
                    .as_u64()
                    .unwrap_or(0);
                assert_eq!(
                    count, expected_by_source[file],
                    "unexpected native work for {file}"
                );
                let id = Some(request.document.project.order[index].0);
                assert_eq!(
                    timings
                        .samples
                        .iter()
                        .filter(|s| s.stage == "coarse_acquire" && s.source == id)
                        .count(),
                    1
                );
                assert!(
                    timings
                        .samples
                        .iter()
                        .filter(|s| s.stage == "fine_acquire" && s.source == id)
                        .count()
                        <= 1
                );
            }
            assert_eq!(approved.entries.len(), paths.len());
            if let Some(reference) = &comparison {
                assert_eq!(reference["roll"], report["roll"]);
                assert_eq!(reference["sources"], report["sources"]);
                assert_eq!(
                    reference["files"], report["files"],
                    "source revisions changed between arms"
                );
                assert_eq!(
                    reference["entries"], report["entries"],
                    "cache warmth changed areas or retained evidence"
                );
            }
            within_limit(start, monitor)
        })();
        scheduler.close_and_wait()?;
        work
    });
    assert_eq!(
        paths
            .iter()
            .map(|p| thumbnails::FileKey::new(p))
            .collect::<Vec<_>>(),
        revisions,
        "source size/mtime changed"
    );
    result.unwrap();
}
