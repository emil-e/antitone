use super::*;
use crate::{coverage::Coverage, geometry::SourceRegion};

fn detected() -> Areas {
    Areas {
        opening: Some([[21., 31.], [101., 31.], [101., 91.], [21., 91.]]),
        picture: Some([[41., 41.], [81., 41.], [81., 81.], [41., 81.]]),
        opening_shared: true,
        opening_exception: true,
        picture_constraints: vec![[1., 0., 40.]],
        picture_weak: [true; 4],
        ..Default::default()
    }
}

#[test]
fn finished_suggestions_round_trip_without_authoring_state_or_a_second_material_map() {
    let original = detected();
    let sampling = SamplingAreas::new(&original);
    let json = serde_json::to_value(&sampling).unwrap();
    for field in ["opening_edited", "picture_edited", "material"] {
        assert!(json.get(field).is_none(), "{field} has a different owner");
    }
    let restored: SamplingAreas = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(restored.areas(None), original);
    let mut authored = json;
    authored["opening_edited"] = serde_json::json!(true);
    assert!(serde_json::from_value::<SamplingAreas>(authored).is_err());
    let mut review = restored.areas(None);
    review.edit_opening(Coverage::rectangle(SourceRegion {
        x: 30,
        y: 40,
        width: 60,
        height: 40,
    }));
    review.edit_picture(Coverage::rectangle(SourceRegion {
        x: 50,
        y: 50,
        width: 20,
        height: 20,
    }));
    assert!(review.opening_edited && review.picture_edited);
    assert_eq!(
        restored.areas(None),
        original,
        "review edits cannot modify the finished result"
    );
}

#[test]
fn persisted_sampling_geometry_is_validated_even_when_picture_framing_is_usable() {
    let active = SourceRegion {
        x: 11,
        y: 17,
        width: 128,
        height: 96,
    };
    SamplingAreas::new(&detected()).validate(active).unwrap();
    let mut invalid = detected();
    invalid.opening.as_mut().unwrap()[0][0] = f64::NAN;
    assert!(SamplingAreas::new(&invalid).validate(active).is_err());
    invalid = detected();
    invalid.opening_exception = true;
    invalid.opening_shared = false;
    assert!(SamplingAreas::new(&invalid).validate(active).is_err());
}
