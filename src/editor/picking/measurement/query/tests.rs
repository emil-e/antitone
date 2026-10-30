use super::*;

#[test]
fn precision_eligibility_keeps_the_strict_floor_boundary_in_the_evaluated_representation() {
    #[derive(Clone)]
    struct Codes {
        domain: evaluation::Domain,
        value: f32,
    }

    impl evaluation::Source for Codes {
        fn domain(&self) -> evaluation::Domain {
            self.domain
        }

        fn read(&self, region: crate::geometry::SourceRegion) -> Result<Vec<[f32; 4]>, Error> {
            Ok(vec![
                [self.value, self.value, self.value, 0.];
                (region.width * region.height) as usize
            ])
        }
    }

    impl evaluation::Operation for Codes {
        fn input(
            &self,
            _: crate::geometry::SourceRegion,
        ) -> Result<Option<crate::geometry::SourceRegion>, Error> {
            Ok(None)
        }

        fn lower(
            &self,
            fragment: &mut evaluation::Fragment,
            _: Option<evaluation::PlannedImage>,
            output: evaluation::PlannedImage,
        ) -> Result<(), Error> {
            fragment.work.push(evaluation::Work::Upload {
                source: Arc::new(self.clone()),
                output,
            });
            Ok(())
        }
    }

    let file = crate::raw::fixture::bayer_dng_with_rgb([2; 3]);
    let editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
    let services = editor.source_services.as_ref().unwrap();
    let mut scan = super::super::capture_scan(&editor).unwrap();
    let coverage = Coverage::rectangle(crate::geometry::SourceRegion {
        x: scan.area.x + 8,
        y: scan.area.y + 8,
        width: 32,
        height: 32,
    });
    assert_eq!(
        scan.effective_floor(),
        crate::processing::CAMERA_INPUT_FLOOR
    );
    let mut samples = Samples::new(services, &mut scan, coverage, &|| false).unwrap();

    for value in [0.5, 1., 2.] {
        samples.source = evaluation::ImageNode::new(
            samples.source.domain,
            None,
            Codes {
                domain: samples.source.domain,
                value,
            },
        );
        let result = samples.require_precision(&|| false);
        if value < 1. {
            assert!(matches!(result, Err(Error::UnusableSample(_))));
        } else {
            result.unwrap();
        }
    }
}
