use vow_perf::{AnalysisError, ComplexityClass, Sample, Verdict, analyze};

fn instrumented_quadratic_logarithmic_work(input_size: u64) -> u64 {
    let mut operations = 0;

    for _ in 0..input_size {
        for _ in 0..input_size {
            let mut remaining = input_size;
            while remaining > 1 {
                operations += 1;
                remaining /= 2;
            }
        }
    }

    operations
}

#[test]
fn quadratic_declaration_rejects_instrumented_n_squared_log_n_work() {
    let samples = [16, 32, 64, 128, 256, 512].map(|input_size| {
        Sample::new(
            input_size,
            instrumented_quadratic_logarithmic_work(input_size),
        )
    });

    let analysis = analyze(ComplexityClass::Quadratic, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Fail);
    assert_eq!(
        analysis.observed,
        Some(ComplexityClass::QuadraticLogarithmic)
    );
}

#[test]
fn canonicalization_extends_each_supported_polynomial_degree_with_one_log_factor() {
    assert_eq!(
        ComplexityClass::from_factors(0, 1),
        Ok(ComplexityClass::Logarithmic)
    );
    assert_eq!(
        ComplexityClass::from_factors(1, 1),
        Ok(ComplexityClass::Linearithmic)
    );
    assert_eq!(
        ComplexityClass::from_factors(2, 1),
        Ok(ComplexityClass::QuadraticLogarithmic)
    );
    assert_eq!(
        ComplexityClass::from_factors(3, 1),
        Ok(ComplexityClass::CubicLogarithmic)
    );
    assert!(ComplexityClass::from_factors(4, 0).is_err());
    assert!(ComplexityClass::from_factors(4, 1).is_err());
    assert!(ComplexityClass::from_factors(0, 2).is_err());
}

#[test]
fn equal_quality_fits_choose_the_simplest_complexity_class() {
    let samples = [16, 32, 64, 128, 256, 512].map(|input_size| Sample::new(input_size, 7));

    let analysis = analyze(ComplexityClass::Constant, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Pass);
    assert_eq!(analysis.observed, Some(ComplexityClass::Constant));
}

#[test]
fn malformed_measurement_grids_are_rejected() {
    assert_eq!(
        analyze(
            ComplexityClass::Linear,
            &[Sample::new(16, 16), Sample::new(32, 32)]
        ),
        Err(AnalysisError::TooFewSamples)
    );
    assert_eq!(
        analyze(
            ComplexityClass::Linear,
            &[Sample::new(1, 1), Sample::new(2, 2), Sample::new(4, 4)]
        ),
        Err(AnalysisError::InputSizeTooSmall {
            index: 0,
            input_size: 1
        })
    );
    assert_eq!(
        analyze(
            ComplexityClass::Linear,
            &[
                Sample::new(16, 16),
                Sample::new(16, 32),
                Sample::new(32, 64)
            ]
        ),
        Err(AnalysisError::NonIncreasingInputSize {
            index: 1,
            previous: 16,
            input_size: 16
        })
    );
}

#[test]
fn log_polynomial_classes_have_class_specific_doubling_ratios() {
    let quadratic = ComplexityClass::Quadratic
        .expected_doubling_ratio(1024)
        .unwrap();
    let quadratic_logarithmic = ComplexityClass::QuadraticLogarithmic
        .expected_doubling_ratio(1024)
        .unwrap();
    let cubic_logarithmic = ComplexityClass::CubicLogarithmic
        .expected_doubling_ratio(1024)
        .unwrap();

    assert!((quadratic - 4.0).abs() < f64::EPSILON);
    assert!((quadratic_logarithmic - 4.4).abs() < 1e-12);
    assert!((cubic_logarithmic - 8.8).abs() < 1e-12);
}
